//! The bridge against the real bundled CLI (SPEC §17, desktop tests).
//!
//! These run the staged sidecar rather than a stand-in, because the properties
//! that matter — that it is *this* binary, launched without a shell, fed
//! Markdown on stdin — are only true if the actual invocation is true.
//!
//! Requires the sidecar to be staged first: `npm run sidecar`.

use std::path::PathBuf;

use serde_json::json;
use heimdall_desktop_lib::cli_bridge::{self, sidecar_path};

fn staged() -> PathBuf {
    isolate_data_dir();
    let path = sidecar_path();
    assert!(
        path.is_file(),
        "the sidecar is not staged at {path:?}; run `npm run sidecar` first"
    );
    path
}

/// Keep the forked sidecars out of the real application-data directory.
///
/// The bridge spawns the CLI without a shell and without touching the
/// environment, so the child inherits this process's. Setting it once, behind a
/// `OnceLock`, is what makes that safe to do from tests running in parallel.
fn isolate_data_dir() {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        std::env::set_var(
            "HEIMDALL_DATA_DIR",
            std::env::temp_dir().join("heimdall-test-data"),
        );
        // Answers the unlock prompt. Only a debug sidecar listens, and only
        // for a vault under the temp directory, which every vault here is.
        std::env::set_var("HEIMDALL_TEST_PRESENCE", "confirm");
    });
}

fn temp_root() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn root_of(dir: &tempfile::TempDir) -> String {
    dir.path().to_str().unwrap().to_string()
}

#[test]
fn the_bundled_sidecar_creates_a_templated_vault() {
    staged();
    let dir = temp_root();

    let response = cli_bridge::run(
        "create",
        &json!({ "name": "demo", "root": root_of(&dir) }),
        None,
    );

    assert!(response.ok, "{response:?}");
    let data = response.data.unwrap();
    assert_eq!(data["mode"], "scaffolded");
    assert_eq!(response.schema_version, Some(1));

    // The vault is real, not just a reported success.
    let vault = dir.path().join("demo");
    assert!(vault.join("ideas/hello_world.md").is_file());
    assert!(!vault.join("aios").exists());
}

#[test]
fn registering_an_existing_folder_leaves_its_notes_alone() {
    staged();
    let dir = temp_root();
    let vault = dir.path().join("existing");
    std::fs::create_dir_all(vault.join(".config")).unwrap();
    std::fs::write(vault.join(".config/app.json"), b"{\"mine\":true}").unwrap();
    std::fs::write(vault.join("note.md"), b"my content").unwrap();

    let response = cli_bridge::run(
        "create",
        &json!({ "name": "existing", "root": root_of(&dir) }),
        None,
    );

    assert!(response.ok, "{response:?}");
    assert_eq!(response.data.unwrap()["mode"], "registered");
    assert_eq!(std::fs::read(vault.join("note.md")).unwrap(), b"my content");
    assert_eq!(
        std::fs::read(vault.join(".config/app.json")).unwrap(),
        b"{\"mine\":true}"
    );
    // No example notes were added to a vault that already had content.
    assert!(!vault.join("ideas").exists());
}

#[test]
fn markdown_travels_through_stdin_and_arrives_unchanged() {
    staged();
    let dir = temp_root();
    let root = root_of(&dir);
    cli_bridge::run("create", &json!({ "name": "demo", "root": root }), None);
    let vault = dir.path().join("demo").to_str().unwrap().to_string();

    let note = "projects/my_project.md";
    let read = cli_bridge::run("read", &json!({ "vault": vault, "path": note }), None);
    let revision = read.data.unwrap()["document"]["revision"].as_str().unwrap().to_string();

    // Quotes, backticks, `$`, and newlines all survive because content is piped
    // rather than passed as an argument (SPEC §11).
    let content = "# Plan\n\n`rm -rf $HOME` \"quoted\" 'single' $(echo hi)\n\n- 🌍\n";
    let written = cli_bridge::run(
        "write",
        &json!({ "vault": vault, "path": note, "expected-revision": revision }),
        Some(content),
    );
    assert!(written.ok, "{written:?}");

    let back = cli_bridge::run("read", &json!({ "vault": vault, "path": note }), None);
    assert_eq!(back.data.unwrap()["document"]["content"], content);
}

#[test]
fn no_shell_ever_interprets_an_argument() {
    staged();
    let dir = temp_root();

    // If any of this reached a shell, the `touch` would run and the folder name
    // would not survive intact.
    let hostile = "demo; touch pwned; echo";
    let response = cli_bridge::run(
        "create",
        &json!({ "name": hostile, "root": root_of(&dir) }),
        None,
    );

    assert!(response.ok, "{response:?}");
    assert!(dir.path().join(hostile).is_dir(), "the name was not taken literally");
    assert!(!dir.path().join("pwned").exists(), "a shell ran the argument");
}

#[test]
fn a_decoy_on_path_is_never_run_in_place_of_the_bundled_tool() {
    staged();
    let dir = temp_root();

    // A `heimdall` earlier on PATH that would answer with a lie.
    let decoy_dir = dir.path().join("bin");
    std::fs::create_dir_all(&decoy_dir).unwrap();
    let decoy = decoy_dir.join("heimdall");
    std::fs::write(
        &decoy,
        "#!/bin/sh\necho '{\"ok\":true,\"data\":{\"cli_version\":\"666.0.0\"},\"meta\":{\"schema_version\":1}}'\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&decoy, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let original = std::env::var("PATH").unwrap_or_default();
    // Safety: this test process is single-threaded with respect to PATH, and
    // the value is restored before returning.
    unsafe {
        std::env::set_var("PATH", format!("{}:{original}", decoy_dir.display()));
    }

    let status = cli_bridge::status();

    unsafe {
        std::env::set_var("PATH", original);
    }

    assert_ne!(
        status.cli_version.as_deref(),
        Some("666.0.0"),
        "the bridge ran a binary from PATH"
    );
    assert!(status.available);
    assert_eq!(status.path, staged().to_string_lossy());
}

#[test]
fn status_reports_the_bundled_binary_and_what_it_says() {
    let path = staged();
    let status = cli_bridge::status();

    assert_eq!(status.path, path.to_string_lossy());
    assert!(status.available, "{status:?}");
    assert!(status.cli_version.is_some());
    assert!(status.core_version.is_some());
    assert_eq!(status.mcp_protocol_version.as_deref(), Some("2025-11-25"));
    assert_eq!(status.output_schema_version, Some(1));
    assert!(status.error.is_none());
}

#[test]
fn a_domain_failure_arrives_structured_rather_than_as_a_crash() {
    staged();
    let dir = temp_root();

    // A note that is not there: the CLI answers with a code and the path, and
    // the bridge must hand that to the UI intact.
    let response = cli_bridge::run(
        "read",
        &json!({ "vault": root_of(&dir), "path": "missing.md" }),
        None,
    );

    assert!(!response.ok);
    let error = response.error.unwrap();
    assert_eq!(error.code, "NOT_FOUND");
    assert_eq!(error.details["path"], "missing.md");
    assert_eq!(response.exit_code, Some(1));
}

#[test]
fn an_unknown_command_never_reaches_a_process() {
    staged();
    let response = cli_bridge::run("mcp", &json!({ "vault": "/v" }), None);

    assert!(!response.ok);
    assert_eq!(response.error.unwrap().code, "INVALID_INPUT");
    // Nothing ran, so there is no exit code to report.
    assert_eq!(response.exit_code, None);
}

#[test]
fn the_health_check_completes_a_real_mcp_handshake_for_the_shared_vaults() {
    staged();
    let dir = temp_root();
    cli_bridge::run(
        "create",
        &json!({ "name": "demo", "root": root_of(&dir) }),
        None,
    );
    let vault = dir.path().join("demo").to_str().unwrap().to_string();
    // Unique, because the test data directory outlives this run.
    let name = format!("health-{}", std::process::id());
    let shared = cli_bridge::run("share", &json!({ "vault": vault, "name": name }), None);
    assert!(shared.ok, "{shared:?}");

    let health = cli_bridge::health_check();

    assert!(health.ok, "{health:?}");
    assert_eq!(health.server_name.as_deref(), Some("heimdall"));
    assert_eq!(health.protocol_version.as_deref(), Some("2025-11-25"));
    let instructions = health.instructions.unwrap_or_default();
    assert!(instructions.contains("`read`"), "{instructions}");
    // The shared vault is named to the client, and its path never is.
    assert!(instructions.contains(&name), "{instructions}");
    assert!(!instructions.contains(&vault), "{instructions}");

    let unshared = cli_bridge::run("unshare", &json!({ "vault": vault }), None);
    assert!(unshared.ok, "{unshared:?}");
}

#[test]
fn the_vaults_command_lists_what_is_shared() {
    staged();
    let dir = temp_root();
    cli_bridge::run("create", &json!({ "name": "demo", "root": root_of(&dir) }), None);
    let vault = dir.path().join("demo").to_str().unwrap().to_string();
    let name = format!("listed-{}", std::process::id());
    assert!(cli_bridge::run("share", &json!({ "vault": vault, "name": name }), None).ok);

    let listed = cli_bridge::run("vaults", &json!({}), None);
    assert!(listed.ok, "{listed:?}");
    let vaults = listed.data.unwrap()["vaults"].as_array().unwrap().clone();
    let entry = vaults
        .iter()
        .find(|entry| entry["name"] == name.as_str())
        .expect("the shared vault is listed");
    assert_eq!(entry["shared"], true);

    assert!(cli_bridge::run("unshare", &json!({ "vault": vault }), None).ok);
}

/// A created vault and its path, for the editor tests below.
fn editor_vault() -> (tempfile::TempDir, String) {
    staged();
    let dir = temp_root();
    let root = root_of(&dir);
    let created = cli_bridge::run("create", &json!({ "name": "demo", "root": root }), None);
    assert!(created.ok, "{created:?}");
    let vault = dir.path().join("demo").to_str().unwrap().to_string();
    (dir, vault)
}

#[test]
fn the_desktop_saves_a_note_and_reads_back_exactly_what_it_wrote() {
    let (_dir, vault) = editor_vault();

    // The awkward characters go through stdin, because an editor is where they
    // actually turn up.
    let content = "---\nlinks:\n  - \"[[profile]]\"\n---\n\n# Note\n\n`$(whoami)` \"quoted\" 🌍\n";
    let written = cli_bridge::run(
        "write",
        &json!({ "vault": vault, "path": "ideas/note.md", "create": true }),
        Some(content),
    );
    assert!(written.ok, "{written:?}");

    let back = cli_bridge::run("read", &json!({ "vault": vault, "path": "ideas/note.md" }), None);
    assert_eq!(back.data.unwrap()["document"]["content"], content);
}

#[test]
fn a_stale_revision_from_the_desktop_is_a_conflict_rather_than_an_overwrite() {
    let (_dir, vault) = editor_vault();
    cli_bridge::run(
        "write",
        &json!({ "vault": vault, "path": "ideas/note.md", "create": true }),
        Some("original\n"),
    );

    let stale = "blake3:0000000000000000000000000000000000000000000000000000000000000000";
    let response = cli_bridge::run(
        "write",
        &json!({ "vault": vault, "path": "ideas/note.md", "expected-revision": stale }),
        Some("clobbered\n"),
    );

    assert!(!response.ok);
    assert_eq!(response.error.unwrap().code, "REVISION_CONFLICT");

    let back = cli_bridge::run("read", &json!({ "vault": vault, "path": "ideas/note.md" }), None);
    assert_eq!(back.data.unwrap()["document"]["content"], "original\n");
}

#[test]
fn deleting_a_note_moves_it_into_the_vaults_trash() {
    let (dir, vault) = editor_vault();
    cli_bridge::run(
        "write",
        &json!({ "vault": vault, "path": "ideas/doomed.md", "create": true }),
        Some("still here\n"),
    );

    let deleted = cli_bridge::run(
        "delete-path",
        &json!({ "vault": vault, "path": "ideas/doomed.md" }),
        None,
    );
    assert!(deleted.ok, "{deleted:?}");
    assert_eq!(
        deleted.data.unwrap()["trashed_to"],
        ".trash/ideas/doomed.md"
    );

    // Recoverable on disk, and absent from the listing the file tree draws.
    let trashed = dir.path().join("demo/.trash/ideas/doomed.md");
    assert_eq!(std::fs::read_to_string(trashed).unwrap(), "still here\n");

    let listed = cli_bridge::run(
        "read",
        &json!({ "vault": vault, "recursive": true }),
        None,
    );
    let text = listed.data.unwrap().to_string();
    assert!(!text.contains("doomed"), "{text}");
    assert!(!text.contains(".trash"), "{text}");
}

#[test]
fn a_lock_set_from_the_desktop_makes_a_note_read_only_everywhere() {
    let (_dir, vault) = editor_vault();
    let note = "ideas/hello_world.md";

    let locked = cli_bridge::run("lock", &json!({ "vault": vault, "path": "ideas" }), None);
    assert!(locked.ok, "{locked:?}");
    assert_eq!(locked.data.unwrap()["locked"], true);

    let read = cli_bridge::run("read", &json!({ "vault": vault, "path": note }), None);
    let data = read.data.unwrap();
    assert_eq!(data["locked"], true);
    assert_eq!(data["locked_at"], "ideas");
    let revision = data["document"]["revision"].as_str().unwrap().to_string();

    let refused = cli_bridge::run(
        "write",
        &json!({ "vault": vault, "path": note, "expected-revision": revision }),
        Some("changed\n"),
    );
    let error = refused.error.unwrap();
    assert_eq!(error.code, "LOCKED");
    assert_eq!(error.details["locked_at"], "ideas");

    // A whole-vault unlock (no path) clears it.
    let unlocked = cli_bridge::run("unlock", &json!({ "vault": vault }), None);
    assert!(unlocked.ok, "{unlocked:?}");
    let written = cli_bridge::run(
        "write",
        &json!({ "vault": vault, "path": note, "expected-revision": revision }),
        Some("changed\n"),
    );
    assert!(written.ok, "{written:?}");
}

#[test]
fn the_link_graph_of_a_templated_vault_arrives_whole() {
    let (_dir, vault) = editor_vault();
    cli_bridge::run(
        "write",
        &json!({ "vault": vault, "path": "ideas/source.md", "create": true }),
        Some("See [[hello_world]] and [[my_project]].\n"),
    );

    let response = cli_bridge::run("link-graph", &json!({ "vault": vault }), None);
    assert!(response.ok, "{response:?}");
    let data = response.data.unwrap();

    let nodes = data["nodes"].as_array().unwrap();
    let paths: Vec<&str> = nodes.iter().map(|n| n["path"].as_str().unwrap()).collect();
    assert!(paths.contains(&"ideas/hello_world.md"), "{paths:?}");
    assert!(nodes.iter().all(|n| n["locked"] == false));

    let index = |path: &str| paths.iter().position(|p| *p == path).unwrap();
    let edges: Vec<(usize, usize)> = data["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["from"].as_u64().unwrap() as usize,
                e["to"].as_u64().unwrap() as usize,
            )
        })
        .collect();
    let source = index("ideas/source.md");
    assert!(edges.contains(&(source, index("ideas/hello_world.md"))));
    assert!(edges.contains(&(source, index("projects/my_project.md"))));
}

#[test]
fn a_large_link_graph_fits_inside_the_bridges_output_limit() {
    // The test that would catch a payload cliff. Edges carry node *indices*
    // rather than paths for exactly this reason: with string endpoints a vault
    // this size would already be most of the way through the limit.
    let (dir, vault) = editor_vault();
    let notes = dir.path().join("demo/notes");
    std::fs::create_dir_all(&notes).unwrap();
    for index in 0..2_000 {
        let body: String = (0..5)
            // Offset by one so no note links to itself: a self-link is not an
            // edge, and five links a note is the point of the fixture.
            .map(|link| format!("[[note_{:06}]]\n", (index + 1 + link * 37) % 2_000))
            .collect();
        std::fs::write(notes.join(format!("note_{index:06}.md")), body).unwrap();
    }

    let response = cli_bridge::run("link-graph", &json!({ "vault": vault }), None);
    assert!(response.ok, "{response:?}");
    let data = response.data.unwrap();
    assert!(data["nodes"].as_array().unwrap().len() >= 2_000);
    assert_eq!(data["edges"].as_array().unwrap().len(), 10_000);
    assert_eq!(data["truncated"]["node_cap_hit"], false);
}

#[test]
fn the_client_operations_are_reachable_but_the_server_is_still_not() {
    staged();
    // Every editor command runs; `mcp` remains something only the client starts.
    for command in [
        "read",
        "write",
        "lock",
        "unlock",
        "create-folder",
        "move-path",
        "delete-path",
        "link-graph",
    ] {
        let response = cli_bridge::run(command, &json!({ "vault": "/nonexistent" }), None);
        // A domain failure means it reached the tool; a refusal would not have.
        assert_ne!(
            response.error.as_ref().map(|e| e.message.as_str()),
            Some("\"{command}\" is not a command this application can run"),
            "{command} was refused"
        );
    }

    let response = cli_bridge::run("mcp", &json!({ "vault": "/tmp" }), None);
    assert!(!response.ok);
    assert_eq!(response.error.unwrap().code, "INVALID_INPUT");
}
