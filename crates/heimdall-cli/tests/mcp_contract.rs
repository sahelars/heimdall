//! Shell/MCP contract and boundary tests (SPEC §17).
//!
//! These drive the real `heimdall mcp` process over stdio with a real MCP
//! client, so anything that corrupted the protocol stream — a stray print, an
//! envelope on stdout — would show up as a failed handshake rather than a
//! passing assertion.

mod common;

use std::collections::HashMap;

use common::{binary, data, envelope, error_code, new_vault, run, run_with_stdin};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::{RoleClient, RunningService, ServiceError};
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::ServiceExt;
use serde_json::{json, Map, Value};

/// Connect a real client to a real server process for one vault.
async fn connect(vault: &str) -> RunningService<RoleClient, ()> {
    let transport = TokioChildProcess::new(tokio::process::Command::new(binary()).configure(
        |command| {
            command
                .arg("mcp")
                .arg("--vault")
                .arg(vault)
                .env("HEIMDALL_DATA_DIR", common::data_dir())
                .env("HEIMDALL_TEST_PRESENCE", "confirm");
        },
    ))
    .expect("spawn heimdall mcp");

    ().serve(transport).await.expect("MCP handshake")
}

fn arguments(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("tool arguments must be an object, got {other}"),
    }
}

async fn call(
    client: &RunningService<RoleClient, ()>,
    tool: &str,
    args: Value,
) -> Result<CallToolResult, ServiceError> {
    client
        .call_tool(CallToolRequestParams::new(tool.to_string()).with_arguments(arguments(args)))
        .await
}

/// The typed payload of a successful call.
async fn structured(client: &RunningService<RoleClient, ()>, tool: &str, args: Value) -> Value {
    let result = call(client, tool, args).await.expect("no protocol error");
    assert_ne!(result.is_error, Some(true), "expected success: {result:?}");
    result
        .structured_content
        .expect("a successful tool result carries structuredContent")
}

/// The structured payload of an expected domain failure.
async fn failure(client: &RunningService<RoleClient, ()>, tool: &str, args: Value) -> Value {
    let result = call(client, tool, args).await.expect("no protocol error");
    assert_eq!(result.is_error, Some(true), "expected failure: {result:?}");
    result
        .structured_content
        .expect("a domain failure carries structured code, message, and details")
}

// ---------------------------------------------------------------------------
// Equivalence
// ---------------------------------------------------------------------------

#[tokio::test]
async fn both_adapters_produce_the_same_domain_outcome_for_reads() {
    let (_tmp, vault) = new_vault();
    let client = connect(&vault).await;

    // The shell wraps its payload in `data`; MCP returns the same payload as
    // structuredContent. Below those two wrappers the domain result is one
    // value produced by one core operation, and it must stay identical.
    let cases: Vec<(&str, Vec<&str>, Value)> = vec![
        ("read", vec!["read"], json!({})),
        ("read", vec!["read", "--recursive"], json!({ "recursive": true })),
        ("read", vec!["read", "ideas"], json!({ "path": "ideas" })),
        (
            "read",
            vec!["read", "ideas/hello_world.md", "--max-lines", "2"],
            json!({ "path": "ideas/hello_world.md", "max_lines": 2 }),
        ),
    ];

    for (tool, mut argv, args) in cases {
        argv.extend(["--vault", vault.as_str()]);
        let shell = data(&run(&argv));
        let mcp = structured(&client, tool, args.clone()).await;
        assert_eq!(shell, mcp, "{tool} {args} disagreed between adapters");
    }

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_lock_set_by_the_user_binds_mcp_and_mcp_cannot_lift_it() {
    let (_tmp, vault) = new_vault();
    let client = connect(&vault).await;
    let note = "ideas/hello_world.md";
    data(&run(&["lock", "ideas", "--vault", &vault]));

    // MCP sees the lock the shell set, and names the rule responsible.
    let read = structured(&client, "read", json!({ "path": note })).await;
    assert_eq!(read["locked"], true);
    assert_eq!(read["locked_at"], "ideas");

    let revision = read["document"]["revision"].clone();
    let refused = failure(
        &client,
        "write",
        json!({ "path": note, "content": "x\n", "expected_revision": revision }),
    )
    .await;
    assert_eq!(refused["code"], "LOCKED");
    assert_eq!(refused["details"]["locked_at"], "ideas");

    // Neither lock tool exists: calling one is a protocol error, not a result,
    // and the lock is still in place afterwards (SPEC §6, §9).
    for (tool, args) in [
        ("unlock", json!({ "path": "ideas" })),
        ("unlock", json!({})),
        ("lock", json!({})),
    ] {
        let attempt = call(&client, tool, args).await;
        assert!(
            matches!(attempt, Err(ServiceError::McpError(_))),
            "{tool} must not be a tool, got {attempt:?}"
        );
    }
    assert_eq!(data(&run(&["read", note, "--vault", &vault]))["locked"], true);
    assert_eq!(data(&run(&["read", "--vault", &vault]))["locked"], false);

    // Only the user lifts it, and then MCP can write again.
    data(&run(&["unlock", "ideas", "--vault", &vault]));
    let written = structured(
        &client,
        "write",
        json!({ "path": note, "content": "x\n", "expected_revision": revision }),
    )
    .await;
    assert!(written["new_revision"].is_string());

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn both_adapters_produce_the_same_domain_outcome_for_failures() {
    let (_tmp, vault) = new_vault();
    let client = connect(&vault).await;
    data(&run(&["lock", "projects", "--vault", &vault]));

    let cases: Vec<(&str, Vec<&str>, Value, &[u8])> = vec![
        ("read", vec!["read", "missing.md"], json!({ "path": "missing.md" }), b""),
        ("read", vec!["read", "--limit", "201"], json!({ "limit": 201 }), b""),
        ("read", vec!["read", ".trash"], json!({ "path": ".trash" }), b""),
        (
            "write",
            vec!["write", "ideas/hello_world.md"],
            json!({ "path": "ideas/hello_world.md", "content": "x\n" }),
            b"x\n",
        ),
        (
            "write",
            vec!["write", "projects/new.md"],
            json!({ "path": "projects/new.md", "content": "x\n" }),
            b"x\n",
        ),
    ];

    for (tool, mut argv, args, stdin) in cases {
        argv.extend(["--vault", vault.as_str()]);
        let shell = envelope(&run_with_stdin(&argv, stdin));
        let mcp = failure(&client, tool, args.clone()).await;

        assert_eq!(shell["error"]["code"], mcp["code"], "{tool} {args} code differs");
        assert_eq!(
            shell["error"]["message"], mcp["message"],
            "{tool} {args} message differs"
        );
        assert_eq!(
            shell["error"]["details"], mcp["details"],
            "{tool} {args} details differ"
        );
    }

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_write_through_one_adapter_is_visible_through_the_other() {
    let (_tmp, vault) = new_vault();
    let client = connect(&vault).await;
    let note = "projects/my_project.md";

    // Write over MCP...
    let revision = structured(&client, "read", json!({ "path": note })).await["document"]["revision"]
        .as_str()
        .unwrap()
        .to_string();
    let written = structured(
        &client,
        "write",
        json!({ "path": note, "content": "# Plan\n\nvia mcp\n", "expected_revision": revision }),
    )
    .await;

    // ...and read it back through the shell.
    let shell = data(&run(&["read", note, "--vault", &vault]));
    assert_eq!(shell["document"]["content"], "# Plan\n\nvia mcp\n");
    assert_eq!(shell["document"]["revision"], written["new_revision"]);

    // A revision taken over MCP is the same token the shell compares against,
    // so a stale write is refused whichever adapter attempts it.
    let stale = run_with_stdin(
        &["write", note, "--vault", &vault, "--expected-revision", &revision],
        b"clobber\n",
    );
    assert_eq!(error_code(&stale), "REVISION_CONFLICT");

    let conflict = failure(
        &client,
        "write",
        json!({ "path": note, "content": "clobber\n", "expected_revision": revision }),
    )
    .await;
    assert_eq!(conflict["code"], "REVISION_CONFLICT");

    // A lock taken at the shell stops the next MCP write.
    data(&run(&["lock", note, "--vault", &vault]));
    let locked = failure(
        &client,
        "write",
        json!({ "path": note, "content": "x\n", "expected_revision": written["new_revision"] }),
    )
    .await;
    assert_eq!(locked["code"], "LOCKED");
    assert_eq!(locked["details"]["locked_at"], note);

    client.cancel().await.unwrap();
}

// ---------------------------------------------------------------------------
// Output shape
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mcp_results_are_typed_and_never_carry_the_shell_envelope() {
    let (_tmp, vault) = new_vault();
    let client = connect(&vault).await;

    let result = call(&client, "read", json!({}))
        .await
        .unwrap();
    let structured = result.structured_content.clone().unwrap();

    for leaked in ["ok", "data", "meta", "schema_version"] {
        assert!(
            structured.get(leaked).is_none(),
            "shell envelope field {leaked} leaked into MCP: {structured}"
        );
    }
    assert!(structured.get("listing").is_some());

    // The shell keeps its envelope for exactly the same operation.
    let shell = envelope(&run(&["read", "--vault", &vault]));
    assert_eq!(shell["ok"], true);
    assert_eq!(shell["meta"]["schema_version"], 1);
    assert_eq!(shell["data"], structured);

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn every_tool_result_validates_against_its_published_output_schema() {
    let (_tmp, vault) = new_vault();
    let client = connect(&vault).await;

    let schemas: HashMap<String, Value> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|tool| {
            let schema = tool
                .output_schema
                .as_ref()
                .unwrap_or_else(|| panic!("{} publishes no output schema", tool.name));
            (tool.name.to_string(), serde_json::to_value(schema).unwrap())
        })
        .collect();

    // Spot-check that each result actually has the top-level shape its schema
    // promises; a full JSON Schema validator is beyond what this needs to catch
    // drift between the declared and returned types.
    for (tool, args) in [
        ("read", json!({})),
        ("read", json!({ "path": "ideas/hello_world.md" })),
        ("write", json!({ "path": "ideas/new.md", "content": "# New\n" })),
    ] {
        let result = structured(&client, tool, args).await;
        let required = schemas[tool]["required"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for field in required {
            let field = field.as_str().unwrap();
            assert!(
                result.get(field).is_some(),
                "{tool} result is missing required field {field}: {result}"
            );
        }
    }

    client.cancel().await.unwrap();
}

// ---------------------------------------------------------------------------
// Failure classification
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_domain_refusal_and_an_unknown_tool_are_different_kinds_of_failure() {
    let (_tmp, vault) = new_vault();
    let client = connect(&vault).await;

    // A refusal the server understood: a tool result flagged isError, carrying
    // a domain code the client can branch on.
    let refused = failure(
        &client,
        "read",
        json!({ "path": "ideas/hello_world.md", "start_line": 0 }),
    )
    .await;
    assert_eq!(refused["code"], "INVALID_INPUT");

    // A call the server cannot execute at all: a protocol error, not a result.
    let unknown = client
        .call_tool(
            CallToolRequestParams::new("read_file")
                .with_arguments(arguments(json!({ "path": "/etc/hosts" }))),
        )
        .await;
    assert!(
        matches!(unknown, Err(ServiceError::McpError(_))),
        "an unknown tool must be a protocol error, got {unknown:?}"
    );

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn arguments_that_violate_a_tool_schema_still_carry_a_domain_code() {
    let (_tmp, vault) = new_vault();
    let client = connect(&vault).await;

    for (tool, args) in [
        ("read", json!({ "limit": "lots" })),
        ("write", json!({ "path": "a.md" })),
    ] {
        let result = failure(&client, tool, args).await;
        assert_eq!(
            result["code"], "INVALID_INPUT",
            "{tool} returned an uncoded failure: {result}"
        );
        assert!(result["message"].as_str().unwrap().contains(tool));
    }

    client.cancel().await.unwrap();
}

// ---------------------------------------------------------------------------
// Vault boundary
// ---------------------------------------------------------------------------

/// Connect a real client to a server for the vaults shared in `data_dir`.
async fn connect_shared(data_dir: &std::path::Path) -> RunningService<RoleClient, ()> {
    let transport = TokioChildProcess::new(tokio::process::Command::new(binary()).configure(
        |command| {
            command.arg("mcp").env("HEIMDALL_DATA_DIR", data_dir);
        },
    ))
    .expect("spawn heimdall mcp");
    ().serve(transport).await.expect("MCP handshake")
}

/// Run the shell against a data directory of the test's own, so the set of
/// shared vaults is exactly what the test made it.
fn run_in(data_dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(binary())
        .env("HEIMDALL_DATA_DIR", data_dir)
        .args(args)
        .output()
        .unwrap()
}

#[tokio::test]
async fn a_call_selects_only_a_shared_vault_and_only_by_name() {
    let data_dir = tempfile::tempdir().unwrap();
    let (_work_tmp, work) = new_vault();
    let (_home_tmp, home) = new_vault();
    let (_private_tmp, private) = new_vault();
    for (vault, name) in [(&work, "Work"), (&home, "Home"), (&private, "Private")] {
        let output = run_in(data_dir.path(), &["share", "--vault", vault, "--name", name]);
        assert!(output.status.success(), "{output:?}");
    }
    assert!(run_in(data_dir.path(), &["unshare", "--vault", &private]).status.success());
    let client = connect_shared(data_dir.path()).await;

    // The one vault input is a name; nothing in a schema takes a location.
    for tool in client.list_all_tools().await.unwrap() {
        let schema = serde_json::to_value(&*tool.input_schema).unwrap();
        assert_eq!(schema["properties"]["vault"]["type"], json!(["string", "null"]));
        let text = schema.to_string();
        for forbidden in ["\"root\"", "\"vault_path\"", "\"vault_id\""] {
            assert!(!text.contains(forbidden), "{} advertises {forbidden}", tool.name);
        }
    }
    let instructions = client.peer_info().unwrap().instructions.clone().unwrap();
    assert!(instructions.contains("\"Home\", \"Work\""), "{instructions}");
    assert!(!instructions.contains("Private"), "{instructions}");

    // With two shared, leaving the vault out is refused with the names.
    let unnamed = failure(&client, "write", json!({ "path": "ideas/x.md", "content": "x\n" })).await;
    assert_eq!(unnamed["code"], "INVALID_INPUT");
    assert_eq!(unnamed["details"]["vaults"], json!(["Home", "Work"]));

    // An unshared vault, a made-up name, and a path are all just names nobody
    // shared — the same answer, which says nothing about what else exists.
    let unshared = failure(&client, "read", json!({ "vault": "Private" })).await;
    let invented = failure(&client, "read", json!({ "vault": "Nowhere" })).await;
    let by_path = failure(&client, "read", json!({ "vault": private.as_str() })).await;
    for refused in [&unshared, &invented, &by_path] {
        assert_eq!(refused["code"], "NOT_FOUND", "{refused}");
        assert_eq!(refused["details"]["vaults"], json!(["Home", "Work"]));
    }

    // A name is matched ignoring case, and the result says which vault it was.
    let written = structured(
        &client,
        "write",
        json!({ "vault": "work", "path": "ideas/served.md", "content": "x\n" }),
    )
    .await;
    assert_eq!(written["vault"], "Work");
    assert_eq!(error_code(&run(&["read", "ideas/served.md", "--vault", &home])), "NOT_FOUND");
    assert_eq!(data(&run(&["read", "ideas/served.md", "--vault", &work]))["document"]["content"], "x\n");

    // An absolute path is read as a path inside the named vault, never as a
    // way out of it.
    let absolute = format!("{home}/ideas/hello_world.md");
    let outside = failure(&client, "read", json!({ "vault": "Work", "path": absolute })).await;
    assert_eq!(outside["code"], "NOT_FOUND");

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn sharing_and_unsharing_take_effect_on_the_next_call() {
    let data_dir = tempfile::tempdir().unwrap();
    let (_tmp, vault) = new_vault();
    let client = connect_shared(data_dir.path()).await;

    // Nothing shared yet: the server is up, and says so per call.
    assert_eq!(failure(&client, "read", json!({})).await["code"], "NOT_FOUND");

    assert!(run_in(data_dir.path(), &["share", "--vault", &vault]).status.success());
    let root = structured(&client, "read", json!({})).await;
    assert_eq!(root["vault"], "demo");

    assert!(run_in(data_dir.path(), &["unshare", "--vault", &vault]).status.success());
    assert_eq!(failure(&client, "read", json!({ "vault": "demo" })).await["code"], "NOT_FOUND");

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_fixed_server_answers_only_to_its_own_name() {
    let (_tmp, vault) = new_vault();
    let client = connect(&vault).await;
    assert_eq!(structured(&client, "read", json!({ "vault": "demo" })).await["vault"], "demo");
    let other = failure(&client, "read", json!({ "vault": "elsewhere" })).await;
    assert_eq!(other["code"], "NOT_FOUND");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn hidden_folders_and_the_trash_stay_unreachable() {
    let (_tmp, vault) = new_vault();
    std::fs::create_dir_all(format!("{vault}/.trash")).unwrap();
    std::fs::write(format!("{vault}/.trash/gone.md"), "gone\n").unwrap();
    let client = connect(&vault).await;

    for path in [".trash", ".trash/gone.md"] {
        assert_eq!(failure(&client, "read", json!({ "path": path })).await["code"], "INVALID_INPUT");
    }
    let write = failure(&client, "write", json!({ "path": ".trash/new.md", "content": "x" })).await;
    assert_eq!(write["code"], "INVALID_INPUT");

    let listed = structured(&client, "read", json!({ "recursive": true, "max_depth": 16 })).await;
    let paths = serde_json::to_string(&listed["listing"]["entries"]).unwrap();
    assert!(!paths.contains(".trash"), "{paths}");

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_vault_that_cannot_be_opened_is_reported_per_call_and_the_server_stays_up() {
    // A server that exited here would leave the client listing no tools and
    // the user with no idea why — what happened when macOS kept the sidecar
    // out of ~/Documents. The reason has to reach the user through a call.
    let client = connect("/nonexistent-heimdall-vault").await;

    let mut names: Vec<_> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect();
    names.sort();
    assert_eq!(names, ["read", "write"]);

    let refused = failure(&client, "read", json!({})).await;
    assert_eq!(refused["code"], "NOT_FOUND", "{refused}");
    let instructions = client.peer_info().unwrap().instructions.clone().unwrap();
    assert!(instructions.contains("could not open it"), "{instructions}");

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn diagnostics_go_to_stderr_without_disturbing_the_protocol() {
    let (_tmp, vault) = new_vault();

    // The handshake below is itself the assertion: the client parses stdout as
    // a strict message stream, so any diagnostic written there would break it.
    let client = connect(&vault).await;
    let info = client.peer_info().expect("server info");

    assert_eq!(info.server_info.as_ref().unwrap().name, "heimdall");
    assert_eq!(info.protocol_version.to_string(), "2025-11-25");
    assert!(info.capabilities.tools.is_some());
    // The instructions field is the protocol's own way of telling a client how
    // to work in this vault, and the only one Heimdall has — there is no
    // instructions file for a client to go and find (SPEC §12).
    let instructions = info.instructions.as_deref().expect("server instructions");
    assert!(instructions.contains("`read`"), "{instructions}");
    assert!(instructions.contains("locked"), "{instructions}");

    let mut names: Vec<_> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect();
    names.sort();
    assert_eq!(names, ["read", "write"]);

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn the_reported_protocol_version_matches_what_the_server_negotiates() {
    let (_tmp, vault) = new_vault();

    let reported = data(&run(&["--version", "--json"]))["mcp_protocol_version"]
        .as_str()
        .unwrap()
        .to_string();

    let client = connect(&vault).await;
    let negotiated = client.peer_info().unwrap().protocol_version.to_string();
    assert_eq!(reported, negotiated);

    client.cancel().await.unwrap();
}
