//! Cross-process locking around revision comparison and replacement
//! (SPEC §14, §17).
//!
//! Optimistic concurrency only holds if the comparison and the write are one
//! indivisible step. These tests race real `heimdall` processes: without the
//! per-file lock, several writers read the same revision, all find it current,
//! and all report success while silently overwriting one another.

mod common;

use std::io::Write;
use std::process::{Child, Command, Stdio};

use common::*;

const WRITERS: usize = 8;

fn spawn_writer(vault: &str, revision: &str, content: &str) -> Child {
    let mut child = Command::new(binary())
        .args(["write-memory", "--vault", vault, "--expected-revision", revision])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(content.as_bytes())
        .unwrap();
    child
}

#[test]
fn concurrent_writers_sharing_one_revision_produce_exactly_one_winner() {
    let (_tmp, vault) = new_vault();
    let revision = main_memory_revision(&vault);

    let children: Vec<_> = (0..WRITERS)
        .map(|index| spawn_writer(&vault, &revision, &format!("# Memory\n\nwriter {index}\n")))
        .collect();

    let mut winners = Vec::new();
    let mut conflicts = 0;
    for child in children {
        let output = child.wait_with_output().unwrap();
        let envelope = envelope(&output);
        if envelope["ok"] == true {
            winners.push(envelope["data"]["new_revision"].as_str().unwrap().to_string());
            assert_eq!(output.status.code(), Some(0));
        } else {
            assert_eq!(
                envelope["error"]["code"], "REVISION_CONFLICT",
                "unexpected failure: {envelope}"
            );
            assert_eq!(output.status.code(), Some(1));
            conflicts += 1;
        }
    }

    assert_eq!(winners.len(), 1, "expected exactly one writer to win");
    assert_eq!(conflicts, WRITERS - 1);

    // The stored file is precisely the winner's, not a blend of several writes.
    let stored = main_memory_revision(&vault);
    assert_eq!(stored, winners[0]);

    let content = data(&run(&["read-memory", "--vault", &vault]))["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(content.starts_with("# Memory\n\nwriter "), "{content:?}");
    assert_eq!(content.lines().count(), 3, "content was interleaved: {content:?}");
}

#[test]
fn concurrent_creates_of_one_extended_memory_leave_a_single_file() {
    let (_tmp, vault) = new_vault();

    let children: Vec<_> = (0..WRITERS)
        .map(|index| {
            let mut child = Command::new(binary())
                .args([
                    "write-memory",
                    "--vault",
                    &vault,
                    "--extended",
                    "topic.md",
                    "--create",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(format!("creator {index}\n").as_bytes())
                .unwrap();
            child
        })
        .collect();

    let successes = children
        .into_iter()
        .filter(|_| true)
        .map(|child| child.wait_with_output().unwrap())
        .filter(|output| envelope(output)["ok"] == true)
        .count();

    assert_eq!(successes, 1, "a create-only write must not race into a replace");

    let memories = data(&run(&["list-memories", "--vault", &vault]));
    let topics = memories["memories"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["name"] == "topic.md")
        .count();
    assert_eq!(topics, 1);
}

#[test]
fn concurrent_entry_creation_never_overwrites_an_existing_entry() {
    let (_tmp, vault) = new_vault();

    let children: Vec<_> = (0..WRITERS)
        .map(|index| {
            let mut child = Command::new(binary())
                .args(["create-entry", "--vault", &vault, "--kind", "conversation"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(format!("# Summary {index}\n").as_bytes())
                .unwrap();
            child
        })
        .collect();

    let ids: Vec<String> = children
        .into_iter()
        .map(|child| child.wait_with_output().unwrap())
        .map(|output| data(&output)["id"].as_str().unwrap().to_string())
        .collect();

    // Entries are create-only: every writer gets its own file, even when they
    // all land in the same second.
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), WRITERS, "ids collided: {ids:?}");

    let listed = data(&run(&["list-entries", "--vault", &vault, "--kind", "conversation"]));
    assert_eq!(listed["entries"].as_array().unwrap().len(), WRITERS);
}
