//! The MCP stdio adapter (SPEC §9, §11, §12).
//!
//! This is a translation layer, not a second implementation: every tool hands
//! its typed request to the same `heimdall-core` operation the shell calls, so
//! the two adapters cannot drift in domain behavior.
//!
//! Two rules shape everything here. Stdout belongs to the protocol, so
//! diagnostics go to stderr. And the vault is fixed by `--vault` at startup —
//! no tool input names, switches, or discovers a vault path.

use std::sync::Arc;

use camino::Utf8Path;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ProtocolVersion,
    ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_handler, tool_router, ErrorData, Json, RoleServer, ServerHandler, ServiceExt};
use serde_json::json;
use heimdall_core::commands::{
    self, CreateEntryRequest, CreateEntryResponse, ListDocumentsRequest, ListDocumentsResponse,
    ListEntriesRequest, ListEntriesResponse, ListMemoriesRequest, ListMemoriesResponse,
    ReadDocumentsRequest, ReadDocumentsResponse, ReadEntryRequest,
    ReadMemoryRequest, ReadResult, WriteMemoryRequest, WriteMemoryResponse,
};
// `Result` is deliberately not imported: the rmcp macros expand bare `Result`
// in generated code, which a domain alias in scope would silently capture.
use heimdall_core::{Error, ErrorCode, Vault};

type CoreResult<T> = heimdall_core::Result<T>;

/// The MCP protocol baseline this build is tested against.
///
/// Pinned deliberately rather than tracking the newest revision (SPEC §12, §21):
/// clients negotiate down to it, and moving it is a deliberate change gated on
/// the contract tests.
pub const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::V_2025_11_25;

/// Server-level guidance, published at initialization (SPEC §12).
///
/// This is how the protocol itself tells a client how to behave in this vault —
/// there is no instructions file for a client to go and find. What varies per
/// vault is the user's own standing preferences, and those live in the main
/// memory, which `read_memory` returns.
const INSTRUCTIONS: &str = "\
Heimdall manages one Markdown vault. Ordinary notes live outside `aios/`; \
protected memories and entries (conversation summaries and notifications) live \
inside it. Read the main memory with `read_memory` at the start of a session: \
it holds the user's durable context and how they want you to work in this \
vault. List content before reading it, request only the ranges needed, and \
follow continuation metadata. Use `write_memory` only for durable memory and \
always pass the revision returned by the latest read. Use `create_entry` for \
conversation summaries and notifications; it never replaces existing files.";

/// One vault, one server process. The handle is shared across concurrent tool
/// calls; the vault it points at is fixed for the process lifetime.
#[derive(Clone)]
pub struct HeimdallServer {
    vault: Arc<Vault>,
    tool_router: ToolRouter<Self>,
}

/// A typed success, or a structured domain failure carried as an error result
/// rather than a protocol error.
///
/// Tool signatures spell this out rather than using the alias: the `#[tool]`
/// macro reads the literal return type to derive each tool's `outputSchema`,
/// and an alias hides the `Json<T>` it looks for.
type ToolResult<T> = std::result::Result<Json<T>, CallToolResult>;

impl HeimdallServer {
    pub fn new(vault: Arc<Vault>) -> Self {
        Self {
            vault,
            tool_router: Self::tool_router(),
        }
    }

    /// Run one core operation and shape its outcome for MCP.
    ///
    /// `heimdall-core` is synchronous and filesystem-bound, so the call goes to a
    /// blocking worker instead of stalling the reactor that is also servicing
    /// cancellation and other in-flight requests.
    async fn run<T, F>(&self, operation: F) -> ToolResult<T>
    where
        F: FnOnce(&Vault) -> CoreResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let vault = Arc::clone(&self.vault);
        match tokio::task::spawn_blocking(move || operation(&vault)).await {
            Ok(Ok(value)) => Ok(Json(value)),
            Ok(Err(error)) => Err(domain_failure(&error)),
            Err(join) => Err(domain_failure(&Error::internal(if join.is_cancelled() {
                "the operation was cancelled"
            } else {
                "the operation failed unexpectedly"
            }))),
        }
    }
}

/// Render an expected domain failure as a tool result with `isError: true` and
/// structured `code`, `message`, and safe `details` (SPEC §11).
///
/// Protocol errors are reserved for what the server genuinely cannot execute —
/// an unknown tool or a malformed request — so a client can tell "your call was
/// understood and refused" from "your call was not understood".
fn domain_failure(error: &Error) -> CallToolResult {
    CallToolResult::structured_error(json!({
        "code": error.code,
        "message": error.message,
        "details": error.details,
    }))
}

/// Give an error result that arrived without a domain code one.
///
/// Only the SDK's own argument-deserialization failures reach here unstructured
/// — every failure this adapter raises goes through [`domain_failure`] — and a
/// request whose arguments do not fit the tool's schema is `INVALID_INPUT`.
fn with_domain_code(tool: &str, response: CallToolResponse) -> CallToolResponse {
    let CallToolResponse::Complete(ref result) = response else {
        return response;
    };
    if result.is_error != Some(true) || result.structured_content.is_some() {
        return response;
    }

    let detail = result
        .content
        .iter()
        .find_map(|block| block.as_text().map(|text| text.text.clone()))
        .unwrap_or_else(|| "the arguments do not match this tool's schema".to_string());
    let detail = detail
        .strip_prefix("failed to deserialize parameters: ")
        .unwrap_or(&detail)
        .to_string();

    CallToolResult::structured_error(json!({
        "code": ErrorCode::InvalidInput,
        "message": format!("invalid arguments for \"{tool}\": {detail}"),
        "details": { "tool": tool },
    }))
    .into()
}

#[tool_router(router = tool_router)]
impl HeimdallServer {
    #[tool(
        name = "list_documents",
        description = "Discover ordinary Markdown notes before choosing what to read. \
                       Returns metadata only — relative path, kind, size, modification time — \
                       never content or revisions. Non-recursive by default; recursive listing \
                       needs `recursive: true` and defaults to depth 4 (maximum 16). Page size \
                       defaults to 50 and caps at 200; when more remains, pass the returned \
                       `next_cursor` back as `cursor`. Lists directories and .md files only. \
                       Cannot see the protected `aios/` tree at any depth — use list_memories \
                       or list_entries for that. Read-only."
    )]
    async fn list_documents(
        &self,
        Parameters(request): Parameters<ListDocumentsRequest>,
    ) -> Result<Json<ListDocumentsResponse>, CallToolResult> {
        self.run(move |vault| commands::list_documents(vault, request))
            .await
    }

    #[tool(
        name = "read_documents",
        description = "Read selected, bounded ranges of ordinary notes found with \
                       list_documents. Takes 1 to 10 explicit Markdown paths; directories are \
                       rejected. Each selection returns at most 200 lines by default (maximum \
                       1000), and the call returns at most 64 KiB of content by default \
                       (maximum 256 KiB). Selections that do not fit the budget come back \
                       marked `returned: false` rather than silently dropped, and a partial \
                       file always reports `complete: false` with `next_line`. Cannot read \
                       anything under `aios/`. Read-only."
    )]
    async fn read_documents(
        &self,
        Parameters(request): Parameters<ReadDocumentsRequest>,
    ) -> Result<Json<ReadDocumentsResponse>, CallToolResult> {
        self.run(move |vault| commands::read_documents(vault, request))
            .await
    }

    #[tool(
        name = "list_memories",
        description = "Discover durable memory files: the main memory first, then extended \
                       memories by filename. Returns metadata only, never content or revisions \
                       — read a memory to get the revision a write will need. Honors `limit` \
                       (default 50, maximum 200) and does not paginate. Read-only."
    )]
    async fn list_memories(
        &self,
        Parameters(request): Parameters<ListMemoriesRequest>,
    ) -> Result<Json<ListMemoriesResponse>, CallToolResult> {
        self.run(move |vault| commands::list_memories(vault, request))
            .await
    }

    #[tool(
        name = "read_memory",
        description = "Read a bounded range of the main memory, or of one extended memory named \
                       by `extended`. `extended` takes a bare filename from list_memories \
                       ending in .md, never a path. Returns the revision to pass to \
                       write_memory. Bounded at 200 lines by default, maximum 1000. Read-only."
    )]
    async fn read_memory(
        &self,
        Parameters(request): Parameters<ReadMemoryRequest>,
    ) -> Result<Json<ReadResult>, CallToolResult> {
        self.run(move |vault| commands::read_memory(vault, request))
            .await
    }

    #[tool(
        name = "write_memory",
        description = "MUTATES CONTENT. Replace the complete main memory, or one extended \
                       memory, with new content. This is a whole-file replacement, not a patch \
                       or an append — send the full text you want stored. Always pass \
                       `expected_revision` from the latest read of that file; pass explicit \
                       null only to create a new extended memory. A stale revision fails with \
                       REVISION_CONFLICT and nothing is written. The main memory is capped at \
                       32 KiB and warns from 24 KiB, at which point durable topic detail should \
                       move into extended memories linked from memory.md; extended memories are \
                       capped at 1 MiB. Do not use this for ordinary notes or append-only logs."
    )]
    async fn write_memory(
        &self,
        Parameters(request): Parameters<WriteMemoryRequest>,
    ) -> Result<Json<WriteMemoryResponse>, CallToolResult> {
        self.run(move |vault| commands::write_memory(vault, request))
            .await
    }

    #[tool(
        name = "list_entries",
        description = "Discover conversation summaries or notifications by metadata, newest \
                       first. `kind` is required and selects which folder is listed. Returns \
                       the filename as `id`, creation time, and size — read a selected entry to \
                       get its content and revision. Honors `limit` (default 50, maximum 200). \
                       Read-only."
    )]
    async fn list_entries(
        &self,
        Parameters(request): Parameters<ListEntriesRequest>,
    ) -> Result<Json<ListEntriesResponse>, CallToolResult> {
        self.run(move |vault| commands::list_entries(vault, request))
            .await
    }

    #[tool(
        name = "read_entry",
        description = "Read one entry selected from list_entries. `id` is a bare filename, \
                       never a path, and `kind` must match the folder the entry lives in. \
                       Bounded at 200 lines by default, maximum 1000, with continuation \
                       metadata when more remains. Read-only."
    )]
    async fn read_entry(
        &self,
        Parameters(request): Parameters<ReadEntryRequest>,
    ) -> Result<Json<ReadResult>, CallToolResult> {
        self.run(move |vault| commands::read_entry(vault, request))
            .await
    }

    #[tool(
        name = "create_entry",
        description = "MUTATES CONTENT. Create a new conversation summary or notification. \
                       Entries are create-only: this never replaces an existing file, and a \
                       same-second collision gets a numeric suffix. Heimdall owns the \
                       frontmatter and adds `created_at` and `type` itself, so content that \
                       begins with its own `---` block is rejected — send the body only. \
                       Summaries should be high-level and compressed, not transcripts. Content \
                       is capped at 1 MiB."
    )]
    async fn create_entry(
        &self,
        Parameters(request): Parameters<CreateEntryRequest>,
    ) -> Result<Json<CreateEntryResponse>, CallToolResult> {
        self.run(move |vault| commands::create_entry(vault, request))
            .await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for HeimdallServer {
    /// Dispatch a tool call, giving every client-caused failure a domain code.
    ///
    /// The router turns arguments that violate a tool's input schema into an
    /// error result carrying bare text. That leaves a client unable to branch on
    /// anything, and unable to tell it apart from a domain refusal, so those
    /// results are reshaped here into the same structured form every other
    /// failure uses (SPEC §11).
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResponse, ErrorData> {
        let tool = request.name.to_string();
        let call = ToolCallContext::new(self, request, context);
        Ok(with_domain_code(&tool, self.tool_router.call(call).await?))
    }

    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(PROTOCOL_VERSION)
            // The default identity comes from the SDK's own build environment,
            // which would announce this server as "rmcp".
            .with_server_info(implementation())
            .with_instructions(INSTRUCTIONS)
    }
}

/// Serve one vault over stdio until the client disconnects.
///
/// An uninitialized vault is not a startup failure: SPEC §7 requires domain
/// tools to answer `NOT_INITIALIZED` with actionable guidance, which lets a
/// client connect and be told what to do rather than seeing the server vanish.
/// A vault path that does not resolve at all is a configuration error, and
/// fails here.
pub fn serve(vault_path: &Utf8Path) -> CoreResult<()> {
    let vault = Arc::new(Vault::open(vault_path)?);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| {
            Error::internal(format!("could not start the async runtime: {}", err.kind()))
        })?;

    runtime.block_on(async move {
        // Diagnostics go to stderr; stdout carries protocol JSON only (SPEC §12).
        eprintln!("heimdall mcp: serving one vault over stdio, protocol {PROTOCOL_VERSION}");

        let service = HeimdallServer::new(vault)
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|err| Error::io_error(format!("MCP server failed to start: {err}")))?;

        service
            .waiting()
            .await
            .map_err(|err| Error::io_error(format!("MCP server stopped: {err}")))?;
        Ok(())
    })
}

/// The identity this server announces.
///
/// The product and its executable are both `heimdall` (SPEC §19), not the
/// `heimdall-cli` package name.
fn implementation() -> Implementation {
    Implementation::new("heimdall", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools() -> Vec<rmcp::model::Tool> {
        let dir = tempfile::tempdir().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let vault = Arc::new(Vault::open(&root).unwrap());
        HeimdallServer::new(vault).tool_router.list_all()
    }

    #[test]
    fn the_surface_is_exactly_the_eight_v1_tools() {
        let mut names: Vec<_> = tools().iter().map(|t| t.name.to_string()).collect();
        names.sort();
        assert_eq!(
            names,
            [
                "create_entry",
                "list_documents",
                "list_entries",
                "list_memories",
                "read_documents",
                "read_entry",
                "read_memory",
                "write_memory",
            ]
        );
    }

    #[test]
    fn the_desktop_client_commands_are_not_reachable_over_mcp() {
        // Editing a note, deleting one, and indexing a whole vault are things a
        // human does at the keyboard — the same category as `heimdall create`
        // (SPEC §7, §9). They are shell commands the desktop calls; the tool
        // surface above stays at eight.
        //
        // This test is a second line of defence, not the first. None of those
        // commands derive `JsonSchema`, so giving one a `#[tool]` would not
        // compile. If this assertion ever starts doing real work, something has
        // been added to the request types that should not have been.
        let names: Vec<_> = tools().iter().map(|t| t.name.to_string()).collect();
        for forbidden in [
            "write_document",
            "create_folder",
            "move_path",
            "delete_path",
            "write_entry",
            "link_graph",
            "write-document",
            "delete-path",
            "link-graph",
        ] {
            assert!(!names.contains(&forbidden.to_string()), "{forbidden} exposed");
        }
    }

    #[test]
    fn vault_creation_is_not_reachable_over_mcp() {
        // Creating a vault is a human setup step, and general file access is
        // never a tool (SPEC §7, §9).
        let names: Vec<_> = tools().iter().map(|t| t.name.to_string()).collect();
        for forbidden in ["create", "create_vault", "read_file", "write_file", "execute"] {
            assert!(!names.contains(&forbidden.to_string()), "{forbidden} exposed");
        }
    }

    #[test]
    fn no_tool_input_can_name_a_vault() {
        // The vault is fixed by server configuration; no call may name, switch,
        // or discover one (SPEC §7).
        for tool in tools() {
            let schema = serde_json::to_string(&tool.input_schema).unwrap();
            for forbidden in ["\"vault\"", "\"root\"", "\"vault_id\"", "\"vault_path\""] {
                assert!(
                    !schema.contains(forbidden),
                    "{} accepts {forbidden}: {schema}",
                    tool.name
                );
            }
        }
    }

    #[test]
    fn every_tool_publishes_an_output_schema() {
        for tool in tools() {
            assert!(
                tool.output_schema.is_some(),
                "{} has no output schema",
                tool.name
            );
        }
    }

    #[test]
    fn every_tool_description_says_what_it_cannot_do_and_whether_it_writes() {
        for tool in tools() {
            let description = tool.description.as_deref().unwrap_or_default();
            assert!(
                description.len() > 120,
                "{} has a thin description",
                tool.name
            );
            let mutates = matches!(tool.name.as_ref(), "write_memory" | "create_entry");
            assert_eq!(
                description.contains("MUTATES CONTENT"),
                mutates,
                "{} misstates whether it mutates",
                tool.name
            );
        }
    }

    #[test]
    fn the_protocol_baseline_is_pinned_not_tracked() {
        // Tracking whatever the SDK calls "latest" would move the wire contract
        // on a dependency bump (SPEC §12).
        assert_eq!(PROTOCOL_VERSION, ProtocolVersion::V_2025_11_25);
    }

    #[test]
    fn server_info_advertises_tools_and_publishes_instructions() {
        let dir = tempfile::tempdir().unwrap();
        let root = camino::Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let server = HeimdallServer::new(Arc::new(Vault::open(&root).unwrap()));
        let info = server.get_info();

        assert_eq!(info.protocol_version, PROTOCOL_VERSION);
        assert!(info.capabilities.tools.is_some());
        assert_eq!(info.server_info.name, "heimdall");
        let instructions = info.instructions.expect("instructions are required");
        assert!(instructions.contains("aios/"));
        assert!(instructions.contains("write_memory"));
    }

    #[test]
    fn a_domain_failure_is_structured_and_flagged_without_leaking_content() {
        let error = Error::revision_conflict("stale")
            .with_detail("path", "aios/memories/memory.md")
            .with_detail("current_revision", "blake3:abc");
        let result = domain_failure(&error);

        assert_eq!(result.is_error, Some(true));
        let structured = result.structured_content.expect("structured error content");
        assert_eq!(structured["code"], "REVISION_CONFLICT");
        assert_eq!(structured["message"], "stale");
        assert_eq!(structured["details"]["current_revision"], "blake3:abc");
        // A text fallback exists for clients that do not render structured content.
        assert!(!result.content.is_empty());
    }
}
