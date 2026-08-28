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
            command.arg("mcp").arg("--vault").arg(vault);
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
        ("list_memories", vec!["list-memories"], json!({})),
        ("read_memory", vec!["read-memory"], json!({})),
        (
            "list_documents",
            vec!["list-documents", "--recursive"],
            json!({ "recursive": true }),
        ),
        (
            "read_documents",
            vec!["read-documents", "--doc", "ideas/hello_world.md"],
            json!({ "documents": [{ "path": "ideas/hello_world.md" }] }),
        ),
        (
            "list_entries",
            vec!["list-entries", "--kind", "conversation"],
            json!({ "kind": "conversation" }),
        ),
    ];

    for (tool, mut argv, args) in cases {
        argv.extend(["--vault", vault.as_str()]);
        let shell = data(&run(&argv));
        let mcp = structured(&client, tool, args).await;
        assert_eq!(shell, mcp, "{tool} disagreed between adapters");
    }

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn both_adapters_produce_the_same_domain_outcome_for_failures() {
    let (_tmp, vault) = new_vault();
    let client = connect(&vault).await;

    let cases: Vec<(&str, Vec<&str>, Value)> = vec![
        (
            "read_entry",
            vec!["read-entry", "--kind", "conversation", "--id", "missing.md"],
            json!({ "kind": "conversation", "id": "missing.md" }),
        ),
        (
            "list_documents",
            vec!["list-documents", "--path", "aios"],
            json!({ "path": "aios" }),
        ),
        (
            "list_documents",
            vec!["list-documents", "--limit", "201"],
            json!({ "limit": 201 }),
        ),
        (
            "read_memory",
            vec!["read-memory", "--extended", "../escape.md"],
            json!({ "extended": "../escape.md" }),
        ),
    ];

    for (tool, mut argv, args) in cases {
        argv.extend(["--vault", vault.as_str()]);
        let shell = envelope(&run(&argv));
        let mcp = failure(&client, tool, args).await;

        assert_eq!(shell["error"]["code"], mcp["code"], "{tool} code differs");
        assert_eq!(
            shell["error"]["message"], mcp["message"],
            "{tool} message differs"
        );
        assert_eq!(
            shell["error"]["details"], mcp["details"],
            "{tool} details differ"
        );
    }

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_write_through_one_adapter_is_visible_through_the_other() {
    let (_tmp, vault) = new_vault();
    let client = connect(&vault).await;

    // Write over MCP...
    let revision = structured(&client, "read_memory", json!({})).await["revision"]
        .as_str()
        .unwrap()
        .to_string();
    let written = structured(
        &client,
        "write_memory",
        json!({ "content": "# Memory\n\nvia mcp\n", "expected_revision": revision }),
    )
    .await;

    // ...and read it back through the shell.
    let shell = data(&run(&["read-memory", "--vault", &vault]));
    assert_eq!(shell["content"], "# Memory\n\nvia mcp\n");
    assert_eq!(shell["revision"], written["new_revision"]);

    // A revision taken over MCP is the same token the shell compares against,
    // so a stale write is refused whichever adapter attempts it.
    let stale = run_with_stdin(
        &["write-memory", "--vault", &vault, "--expected-revision", &revision],
        b"clobber\n",
    );
    assert_eq!(error_code(&stale), "REVISION_CONFLICT");

    let conflict = failure(
        &client,
        "write_memory",
        json!({ "content": "clobber\n", "expected_revision": revision }),
    )
    .await;
    assert_eq!(conflict["code"], "REVISION_CONFLICT");

    client.cancel().await.unwrap();
}

// ---------------------------------------------------------------------------
// Output shape
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mcp_results_are_typed_and_never_carry_the_shell_envelope() {
    let (_tmp, vault) = new_vault();
    let client = connect(&vault).await;

    let result = call(&client, "list_memories", json!({}))
        .await
        .unwrap();
    let structured = result.structured_content.clone().unwrap();

    for leaked in ["ok", "data", "meta", "schema_version"] {
        assert!(
            structured.get(leaked).is_none(),
            "shell envelope field {leaked} leaked into MCP: {structured}"
        );
    }
    assert!(structured.get("memories").is_some());

    // The shell keeps its envelope for exactly the same operation.
    let shell = envelope(&run(&["list-memories", "--vault", &vault]));
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
        ("list_memories", json!({})),
        ("read_memory", json!({})),
        ("list_entries", json!({ "kind": "notification" })),
        ("list_documents", json!({})),
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
    let refused = failure(&client, "read_memory", json!({ "start_line": 0 })).await;
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
        ("list_documents", json!({ "limit": "lots" })),
        ("list_entries", json!({})),
        ("read_memory", json!({ "unexpected": true })),
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

#[tokio::test]
async fn no_tool_call_can_name_or_switch_the_vault() {
    let (_tmp, vault) = new_vault();
    let (_other_tmp, other_vault) = new_vault();
    let client = connect(&vault).await;

    // Mark the two vaults apart.
    let revision = structured(&client, "read_memory", json!({})).await["revision"]
        .as_str()
        .unwrap()
        .to_string();
    structured(
        &client,
        "write_memory",
        json!({ "content": "# Served vault\n", "expected_revision": revision }),
    )
    .await;

    // No tool advertises a vault input...
    for tool in client.list_all_tools().await.unwrap() {
        let schema = serde_json::to_string(&tool.input_schema).unwrap();
        for forbidden in ["\"vault\"", "\"root\"", "\"vault_path\"", "\"vault_id\""] {
            assert!(
                !schema.contains(forbidden),
                "{} advertises {forbidden}",
                tool.name
            );
        }
    }

    // ...and supplying one anyway is refused rather than honored.
    let refused = failure(
        &client,
        "read_memory",
        json!({ "vault": other_vault.as_str() }),
    )
    .await;
    assert_eq!(refused["code"], "INVALID_INPUT");

    // The other vault is untouched and the served one still holds its content.
    let other = data(&run(&["read-memory", "--vault", &other_vault]));
    assert_ne!(other["content"], "# Served vault\n");
    assert_eq!(
        structured(&client, "read_memory", json!({})).await["content"],
        "# Served vault\n"
    );

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn protected_content_stays_unreachable_through_document_tools() {
    let (_tmp, vault) = new_vault();
    let client = connect(&vault).await;

    for args in [
        json!({ "path": "aios" }),
        json!({ "path": "AIOS" }),
        json!({ "path": "aios/memories" }),
    ] {
        assert_eq!(failure(&client, "list_documents", args).await["code"], "NOT_FOUND");
    }

    for path in ["aios/memories/memory.md", "AIOS/memories/memory.md"] {
        let refused = failure(
            &client,
            "read_documents",
            json!({ "documents": [{ "path": path }] }),
        )
        .await;
        assert_eq!(refused["code"], "NOT_FOUND", "{path}");
    }

    // A recursive listing never surfaces it either.
    let listed = structured(
        &client,
        "list_documents",
        json!({ "recursive": true, "max_depth": 16 }),
    )
    .await;
    let paths = serde_json::to_string(&listed["entries"]).unwrap();
    assert!(!paths.contains("aios"), "{paths}");

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn an_uninitialized_vault_connects_and_explains_itself() {
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().to_str().unwrap().to_string();

    // The server still starts: a client that connects should be told what is
    // wrong rather than watching the process disappear (SPEC §7).
    let client = connect(&empty).await;

    for (tool, args) in [
        ("list_documents", json!({})),
        ("read_memory", json!({})),
        ("list_memories", json!({})),
        ("list_entries", json!({ "kind": "conversation" })),
    ] {
        let result = failure(&client, tool, args).await;
        assert_eq!(result["code"], "NOT_INITIALIZED", "{tool}");
        assert!(
            result["message"].as_str().unwrap().contains("heimdall create"),
            "{tool} gives no actionable guidance: {result}"
        );
    }

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_vault_path_that_does_not_exist_fails_at_startup_on_stderr() {
    // A missing vault is a configuration error, not something a tool call can
    // report, and it must not put a shell envelope on the protocol stream.
    let output = run(&["mcp", "--vault", "/nonexistent-heimdall-vault"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stdout.is_empty(),
        "stdout must stay protocol-only: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("NOT_FOUND"), "{stderr}");
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
    assert!(instructions.contains("aios/"));
    assert!(instructions.contains("List content before reading it"));
    assert!(instructions.contains("read_memory"), "{instructions}");

    assert_eq!(client.list_all_tools().await.unwrap().len(), 8);

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
