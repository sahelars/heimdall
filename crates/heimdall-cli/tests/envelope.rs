//! The shell output contract (SPEC §11).

mod common;

use common::*;

#[test]
fn a_successful_command_prints_the_versioned_ok_envelope() {
    let (_tmp, vault) = new_vault();
    let output = run(&["list-memories", "--vault", &vault]);

    let envelope = envelope(&output);
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["meta"]["schema_version"], 1);
    assert!(envelope["data"]["memories"].is_array());
    assert!(envelope.get("error").is_none());
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn a_domain_failure_prints_the_error_envelope_and_exits_one() {
    let (_tmp, vault) = new_vault();
    let output = run(&["read-entry", "--vault", &vault, "--kind", "conversation", "--id", "missing.md"]);

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
    let output = run(&["list-documents"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}

#[test]
fn stdout_carries_only_the_envelope() {
    let (_tmp, vault) = new_vault();
    let output = run(&["read-memory", "--vault", &vault]);

    // Parsing the whole of stdout as one JSON value proves nothing else leaked
    // into it — the discipline the MCP transport will depend on in Phase 2.
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

    let created = run_with_stdin(
        &["create-entry", "--vault", &vault, "--kind", "notification"],
        content.as_bytes(),
    );
    let id = data(&created)["id"].as_str().unwrap().to_string();

    let read = run(&["read-entry", "--vault", &vault, "--kind", "notification", "--id", &id]);
    let stored = data(&read)["content"].as_str().unwrap().to_string();
    assert!(stored.ends_with(content), "{stored}");
}

#[test]
fn non_utf8_stdin_is_rejected() {
    let (_tmp, vault) = new_vault();
    let output = run_with_stdin(
        &["create-entry", "--vault", &vault, "--kind", "notification"],
        &[0x23, 0x20, 0xff, 0xfe],
    );
    assert_eq!(error_code(&output), "INVALID_INPUT");
}

#[test]
fn write_memory_round_trips_a_revision_across_processes() {
    let (_tmp, vault) = new_vault();
    let revision = main_memory_revision(&vault);

    let written = run_with_stdin(
        &["write-memory", "--vault", &vault, "--expected-revision", &revision],
        b"# Memory\n\nFresh.\n",
    );
    let new_revision = data(&written)["new_revision"].as_str().unwrap().to_string();
    assert_ne!(new_revision, revision);
    assert_eq!(main_memory_revision(&vault), new_revision);

    // The old revision is now stale.
    let stale = run_with_stdin(
        &["write-memory", "--vault", &vault, "--expected-revision", &revision],
        b"clobber\n",
    );
    assert_eq!(error_code(&stale), "REVISION_CONFLICT");
}

#[test]
fn create_flag_expresses_the_null_revision_for_a_new_extended_memory() {
    let (_tmp, vault) = new_vault();

    let created = run_with_stdin(
        &["write-memory", "--vault", &vault, "--extended", "topic.md", "--create"],
        b"# Topic\n",
    );
    assert_eq!(data(&created)["created"], true);

    // --create against an existing file is a conflict, not an overwrite.
    let again = run_with_stdin(
        &["write-memory", "--vault", &vault, "--extended", "topic.md", "--create"],
        b"clobber\n",
    );
    assert_eq!(error_code(&again), "REVISION_CONFLICT");
}

#[test]
fn an_uninitialized_vault_reports_how_to_fix_itself() {
    let tmp = tempfile::tempdir().unwrap();
    let empty = tmp.path().to_str().unwrap();

    let output = run(&["list-documents", "--vault", empty]);
    assert_eq!(error_code(&output), "NOT_INITIALIZED");
    let message = envelope(&output)["error"]["message"].as_str().unwrap().to_string();
    assert!(message.contains("heimdall create"), "{message}");
}

#[test]
fn no_command_can_name_a_vault_path_in_its_payload() {
    let (_tmp, vault) = new_vault();
    // Paths in a request are vault-relative; an absolute one cannot escape.
    let output = run(&["read-documents", "--vault", &vault, "--doc", "/etc/hosts.md"]);
    let code = error_code(&output);
    assert!(code == "NOT_FOUND" || code == "INVALID_INPUT", "{code}");
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
