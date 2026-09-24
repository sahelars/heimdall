//! Cross-process locking around revision comparison and replacement
//! (SPEC §14, §17).
//!
//! Optimistic concurrency only holds if the comparison and the write are one
//! indivisible step. These tests race real `heimdall` processes: without the
//! vault's write lock, several writers read the same revision, all find it
//! current, and all report success while silently overwriting one another.

mod common;

use std::io::Write;
use std::process::{Child, Stdio};

use common::*;

const WRITERS: usize = 8;
const NOTE: &str = "projects/my_project.md";

fn spawn_write(vault: &str, args: &[&str], content: &str) -> Child {
    let mut child = common::command()
        .args(["write", "--vault", vault])
        .args(args)
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
    let revision = revision_of(&vault, NOTE);

    let children: Vec<_> = (0..WRITERS)
        .map(|index| {
            spawn_write(
                &vault,
                &[NOTE, "--expected-revision", &revision],
                &format!("# My Project\n\nwriter {index}\n"),
            )
        })
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
    assert_eq!(revision_of(&vault, NOTE), winners[0]);
    let content = data(&run(&["read", NOTE, "--vault", &vault]))["document"]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(content.starts_with("# My Project\n\nwriter "), "{content:?}");
    assert_eq!(content.lines().count(), 3, "content was interleaved: {content:?}");
}

#[test]
fn concurrent_creates_of_one_note_leave_a_single_file() {
    let (_tmp, vault) = new_vault();

    let children: Vec<_> = (0..WRITERS)
        .map(|index| spawn_write(&vault, &["ideas/topic.md"], &format!("creator {index}\n")))
        .collect();

    let successes = children
        .into_iter()
        .map(|child| child.wait_with_output().unwrap())
        .filter(|output| envelope(output)["ok"] == true)
        .count();

    assert_eq!(successes, 1, "a create must not race into a replace");
}

#[test]
fn a_lock_taken_by_one_process_binds_every_other() {
    let (_tmp, vault) = new_vault();
    let revision = revision_of(&vault, NOTE);
    data(&run(&["lock", "projects", "--vault", &vault]));

    let refused = spawn_write(&vault, &[NOTE, "--expected-revision", &revision], "x\n")
        .wait_with_output()
        .unwrap();
    assert_eq!(error_code(&refused), "LOCKED");

    data(&run(&["unlock", "projects", "--vault", &vault]));
    let accepted = spawn_write(&vault, &[NOTE, "--expected-revision", &revision], "x\n")
        .wait_with_output()
        .unwrap();
    assert_eq!(data(&accepted)["created"], false);
}
