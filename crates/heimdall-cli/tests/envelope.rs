//! The shell output contract (SPEC §11).

mod common;

use common::*;

#[test]
fn a_successful_command_prints_the_versioned_ok_envelope() {
    let (_tmp, vault) = new_vault();
    let output = run(&["read", "--vault", &vault]);

    let envelope = envelope(&output);
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["meta"]["schema_version"], 1);
    assert!(envelope["data"]["listing"]["entries"].is_array());
    assert!(envelope.get("error").is_none());
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn a_domain_failure_prints_the_error_envelope_and_exits_one() {
    let (_tmp, vault) = new_vault();
    let output = run(&["read", "missing.md", "--vault", &vault]);

    let envelope = envelope(&output);
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"]["code"], "NOT_FOUND");
    assert!(envelope["error"]["message"].as_str().unwrap().len() > 10);
    assert!(envelope["error"]["details"].is_object());
    assert_eq!(envelope["meta"]["schema_version"], 1);
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn a_usage_error_is_distinct_from_a_domain_failure() {
    // Exit 2 with no envelope: the request never reached the domain.
    let output = run(&["write"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}

#[test]
fn stdout_carries_only_the_envelope() {
    let (_tmp, vault) = new_vault();
    let output = run(&["read", "ideas/hello_world.md", "--vault", &vault]);

    // Parsing the whole of stdout as one JSON value proves nothing else leaked
    // into it — the discipline the MCP transport depends on.
    let _ = envelope(&output);
    assert!(
        output.stderr.is_empty(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn running_bare_prints_help_and_mutates_nothing() {
    let (tmp, _vault) = new_vault();
    let before = walk(tmp.path());

    let output = run(&[]);
    assert_eq!(output.status.code(), Some(0));
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("Usage"), "{help}");
    assert!(help.contains("create"), "{help}");

    assert_eq!(walk(tmp.path()), before);
}

#[test]
fn markdown_reaches_the_vault_verbatim_through_stdin() {
    let (_tmp, vault) = new_vault();
    // Quotes, backticks, `$`, and newlines survive because content is never a
    // shell-interpreted argument (SPEC §11).
    let content = "# Notes\n\n`rm -rf $HOME` \"quoted\" 'single' $(echo hi)\n\n- 🌍\n";

    let created = run_with_stdin(&["write", "ideas/verbatim.md", "--vault", &vault], content.as_bytes());
    assert_eq!(data(&created)["created"], true);

    let read = run(&["read", "ideas/verbatim.md", "--vault", &vault]);
    let stored = data(&read)["document"]["content"].as_str().unwrap().to_string();
    assert_eq!(stored, content);
}

#[test]
fn non_utf8_stdin_is_rejected() {
    let (_tmp, vault) = new_vault();
    let output = run_with_stdin(&["write", "ideas/bad.md", "--vault", &vault], &[0x23, 0x20, 0xff, 0xfe]);
    assert_eq!(error_code(&output), "INVALID_INPUT");
}

#[test]
fn write_round_trips_a_revision_across_processes() {
    let (_tmp, vault) = new_vault();
    let note = "projects/my_project.md";
    let revision = revision_of(&vault, note);

    let written = run_with_stdin(
        &["write", note, "--vault", &vault, "--expected-revision", &revision],
        b"# My Project\n\nFresh.\n",
    );
    let new_revision = data(&written)["new_revision"].as_str().unwrap().to_string();
    assert_ne!(new_revision, revision);
    assert_eq!(revision_of(&vault, note), new_revision);

    // The old revision is now stale.
    let stale = run_with_stdin(
        &["write", note, "--vault", &vault, "--expected-revision", &revision],
        b"clobber\n",
    );
    assert_eq!(error_code(&stale), "REVISION_CONFLICT");
}

#[test]
fn a_write_without_a_revision_creates_and_never_replaces() {
    let (_tmp, vault) = new_vault();

    let created = run_with_stdin(&["write", "ideas/topic.md", "--vault", &vault], b"# Topic\n");
    assert_eq!(data(&created)["created"], true);

    for args in [
        vec!["write", "ideas/topic.md", "--vault", &vault],
        vec!["write", "ideas/topic.md", "--vault", &vault, "--create"],
    ] {
        let again = run_with_stdin(&args, b"clobber\n");
        assert_eq!(error_code(&again), "REVISION_CONFLICT");
    }
}

#[test]
fn a_folder_in_no_vault_reports_how_to_fix_itself() {
    let tmp = tempfile::tempdir().unwrap();
    let output = common::command()
        .args(["read"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert_eq!(error_code(&output), "NOT_INITIALIZED");
    let message = envelope(&output)["error"]["message"].as_str().unwrap().to_string();
    assert!(message.contains("--vault"), "{message}");
}

#[test]
fn inside_a_vault_the_four_verbs_need_no_vault_and_take_the_current_folder() {
    let (_tmp, vault) = new_vault();
    let projects = format!("{vault}/projects");
    let here = |args: &[&str], stdin: &[u8]| {
        use std::io::Write;
        let mut child = common::command()
            .args(args)
            .current_dir(&projects)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(stdin).unwrap();
        child.wait_with_output().unwrap()
    };

    // `read` with no path is the folder the shell is standing in.
    let listing = data(&here(&["read"], b""));
    assert_eq!(listing["path"], "projects");
    assert_eq!(listing["listing"]["entries"][0]["path"], "projects/my_project.md");

    // A bare filename is in the current folder; `..` reaches the rest of the vault.
    assert_eq!(data(&here(&["read", "my_project.md"], b""))["kind"], "document");
    assert_eq!(data(&here(&["read", "../ideas/hello_world.md"], b""))["path"], "ideas/hello_world.md");
    assert_eq!(data(&here(&["write", "new.md"], b"# New\n"))["path"], "projects/new.md");

    // `lock` with no path locks the current folder, and a write inside it fails.
    let locked = data(&here(&["lock"], b""));
    assert_eq!((locked["path"].as_str(), locked["locked"].as_bool()), (Some("projects"), Some(true)));
    let refused = here(&["write", "other.md"], b"# Other\n");
    assert_eq!(error_code(&refused), "LOCKED");
    assert_eq!(envelope(&refused)["error"]["details"]["locked_at"], "projects");

    data(&here(&["unlock", "."], b""));
    data(&here(&["write", "other.md"], b"# Other\n"));

    // The whole path works too.
    let absolute = format!("{projects}/other.md");
    assert_eq!(data(&here(&["read", &absolute], b""))["path"], "projects/other.md");
    assert_eq!(error_code(&here(&["read", "../../outside.md"], b"")), "PATH_OUTSIDE_VAULT");
}

#[test]
fn no_command_can_reach_outside_the_vault_through_a_path() {
    let (_tmp, vault) = new_vault();
    // With --vault, a path is vault-relative; an absolute one outside it is refused.
    let output = run(&["read", "/etc/hosts", "--vault", &vault]);
    assert_eq!(error_code(&output), "PATH_OUTSIDE_VAULT");
}

#[test]
fn relink_follows_a_move_through_the_real_binary() {
    // The desktop's actual sequence, over two processes and a real vault: the
    // move renames the file, and the follow-up call carries the links.
    let (_tmp, vault) = new_vault();
    std::fs::write(format!("{vault}/roadmap.md"), "# Roadmap\n").unwrap();
    std::fs::write(
        format!("{vault}/source.md"),
        "See [[roadmap]] and [[roadmap|it]].\n",
    )
    .unwrap();

    let moved = run(&["move-path", "--vault", &vault, "--from", "roadmap.md", "--to", "plan.md"]);
    assert_eq!(moved.status.code(), Some(0));

    let output = run(&["relink", "--vault", &vault, "--from", "roadmap.md", "--to", "plan.md"]);
    let envelope = envelope(&output);
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["meta"]["schema_version"], 1);
    assert_eq!(envelope["data"]["updated"][0]["path"], "source.md");
    assert_eq!(envelope["data"]["updated"][0]["links"], 2);
    assert!(envelope["data"]["skipped"].as_array().unwrap().is_empty());
    assert_eq!(output.status.code(), Some(0));

    let source = std::fs::read_to_string(format!("{vault}/source.md")).unwrap();
    assert_eq!(source, "See [[plan]] and [[plan|it]].\n");
}

#[test]
fn version_reports_component_and_schema_versions() {
    let output = run(&["--version", "--json"]);
    let data = data(&output);

    assert!(data["cli_version"].is_string());
    assert!(data["core_version"].is_string());
    assert_eq!(data["output_schema_version"], 1);
    // The pinned MCP baseline, so a client or the desktop bridge can check
    // compatibility without starting a server.
    assert_eq!(data["mcp_protocol_version"], "2025-11-25");
}

/// Every path under `root`, sorted, for before/after comparison.
fn walk(root: &std::path::Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path.clone());
            }
            found.push(path.to_string_lossy().to_string());
        }
    }
    found.sort();
    found
}

#[test]
fn sharing_names_a_vault_for_ai_clients_and_vaults_lists_it() {
    let data_dir = tempfile::tempdir().unwrap();
    let (_tmp, vault) = new_vault();
    let run_in = |args: &[&str]| {
        common::command()
            .env("HEIMDALL_DATA_DIR", data_dir.path())
            .args(args)
            .output()
            .unwrap()
    };

    let shared = data(&run_in(&["share", "--vault", &vault, "--name", "Work"]));
    assert_eq!(shared["name"], "Work");

    let listed = data(&run_in(&["vaults"]));
    let entry = listed["vaults"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["shared"] == true)
        .cloned()
        .expect("the shared vault is listed");
    assert_eq!(entry["name"], "Work");
    assert_eq!(entry["folder"], "demo");
    assert_eq!(entry["exists"], true);

    // A read names the vault by its shared name.
    assert_eq!(data(&run_in(&["read", "--vault", &vault]))["vault"], "Work");

    let unshared = data(&run_in(&["unshare", "--vault", &vault]));
    assert_eq!(unshared["changed"], true);
    let listed = data(&run_in(&["vaults"]));
    assert!(listed["vaults"].as_array().unwrap().iter().all(|v| v["shared"] == false));
    assert_eq!(data(&run_in(&["read", "--vault", &vault]))["vault"], "demo");
}
