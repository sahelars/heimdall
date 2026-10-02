//! The MCP stdio adapter (SPEC §9, §11, §12).
//!
//! This is a translation layer, not a second implementation: every tool hands
//! its typed request to the same `heimdall-core` operation the shell calls, so
//! the two adapters cannot drift in domain behavior.
//!
//! Two rules shape everything here. Stdout belongs to the protocol, so
//! diagnostics go to stderr. And no tool input ever names a filesystem path to
//! a vault: the server serves either the vaults the user has shared with AI
//! clients, picked per call by **name**, or the one vault `--vault` fixed at
//! startup.
//!
//! The server starts whatever state those vaults are in. A vault macOS will
//! not let this process open, or one that has gone, is reported by the call
//! that needs it — a server that exited instead would leave the client showing
//! no tools at all and the user with nothing to go on.

use std::sync::Arc;

use camino::{Utf8Path, Utf8PathBuf};
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
use heimdall_core::commands::{self, ReadRequest, ReadResponse, WriteRequest, WriteResponse};
// `Result` is deliberately not imported: the rmcp macros expand bare `Result`
// in generated code, which a domain alias in scope would silently capture.
use heimdall_core::{agents, appdata, Error, ErrorCode, Vault};

type CoreResult<T> = heimdall_core::Result<T>;

/// The MCP protocol baseline this build is tested against.
///
/// Pinned deliberately rather than tracking the newest revision (SPEC §12, §21):
/// clients negotiate down to it, and moving it is a deliberate change gated on
/// the contract tests.
pub const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::V_2025_11_25;

/// What every session is told, whichever vaults it serves (SPEC §12).
///
/// This is how the protocol itself tells a client how to behave — there is no
/// instructions file for a client to go and find. Some clients never show it
/// to the model, which is why the guardrail is repeated in the `write`
/// description and in every `LOCKED` refusal.
const INSTRUCTIONS: &str = "\
Start with `read` and no path: it lists the vault root. `read` on a folder lists it; \
`read` on a note returns a bounded range of its lines and the revision a write needs. \
Request only the ranges you need and follow `next_line` and `next_cursor`. `write` \
creates a note, or replaces one when you pass the `expected_revision` from your latest \
read; it never overwrites anything silently. Use these two tools, not the filesystem, \
for anything in these vaults.\n\n\
Every read says whether a path is `locked`. A locked note or folder is read-only by the \
user's decision, and only the user can change that. Never try to change a locked note \
or folder any other way: not with a shell, file-editing or patch tools, or scripts, and \
not by operating the Heimdall app or any other application through computer use, \
accessibility, or the screen. If a write is refused with LOCKED, tell the user what is \
locked and which lock is responsible (`locked_at`), and stop.";

/// Which vaults this process serves.
enum Served {
    /// `heimdall mcp --vault P`: one vault, fixed at startup.
    Fixed(Arc<Vault>),
    /// `heimdall mcp`: the vaults shared with AI clients, read afresh on every
    /// call so sharing or unsharing one takes effect without a restart.
    Shared { data_dir: Utf8PathBuf },
    /// `heimdall mcp --vault P` where P could not be opened at startup —
    /// macOS privacy settings, usually. Each call tries again, so granting
    /// access fixes it without restarting the client.
    Unopened { path: Utf8PathBuf },
}

impl Served {
    /// The vault a call means.
    ///
    /// Shared vaults are opened per call rather than cached: opening is one
    /// `open(2)` and a `realpath`, and a cached handle would follow a folder
    /// that was moved, or keep serving one that was unshared.
    fn open(&self, name: Option<&str>) -> CoreResult<Arc<Vault>> {
        match self {
            Self::Fixed(vault) => Ok(Arc::clone(vault)),
            Self::Unopened { path } => Ok(Arc::new(Vault::open(path)?)),
            Self::Shared { data_dir } => {
                let shared = agents::select(data_dir, name)?;
                Ok(Arc::new(Vault::open_with_data_dir(&shared.path, data_dir)?))
            }
        }
    }

    /// The opening of this session's instructions: which vaults it serves.
    fn describe(&self) -> String {
        match self {
            Self::Fixed(vault) => {
                let name = agents::name_of(vault)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| vault.name().to_string());
                format!(
                    "Heimdall serves one Markdown vault here, \"{name}\", through two tools: \
                     `read` and `write`. `vault` may be left out; if given, it must be \"{name}\"."
                )
            }
            Self::Unopened { .. } => "Heimdall serves one Markdown vault here through two \
                tools, `read` and `write`, but could not open it when this session started; \
                each call reports why, so pass that on to the user."
                .to_string(),
            Self::Shared { data_dir } => match agents::list(data_dir) {
                Ok(vaults) if vaults.len() == 1 => format!(
                    "Heimdall serves the user's Markdown vaults through two tools: `read` and \
                     `write`. One vault is shared: \"{}\". `vault` may be left out while it is \
                     the only one; if more are shared later, a call without it is refused with \
                     the list of names.",
                    vaults[0].name
                ),
                Ok(vaults) if !vaults.is_empty() => format!(
                    "Heimdall serves the user's Markdown vaults through two tools: `read` and \
                     `write`. The vaults shared when this session started: {}. Pass `vault` \
                     with one of these names on every call, and check it is the vault the user \
                     means before writing; a call without it is refused with the current list.",
                    vaults
                        .iter()
                        .map(|vault| format!("\"{}\"", vault.name))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                _ => "Heimdall serves the user's Markdown vaults through two tools: `read` and \
                      `write`. No vault was shared when this session started; until the user \
                      shares one in Heimdall, every call says so."
                    .to_string(),
            },
        }
    }
}

/// One server process: one fixed vault, or the shared set. The handle is
/// shared across concurrent tool calls.
#[derive(Clone)]
pub struct HeimdallServer {
    served: Arc<Served>,
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
    /// Serve one vault, fixed for the life of the process.
    pub fn new(vault: Arc<Vault>) -> Self {
        Self::serving(Served::Fixed(vault))
    }

    /// Serve the vaults shared with AI clients, by name.
    pub fn shared(data_dir: Utf8PathBuf) -> Self {
        Self::serving(Served::Shared { data_dir })
    }

    fn serving(served: Served) -> Self {
        Self {
            served: Arc::new(served),
            tool_router: Self::tool_router(),
        }
    }

    /// Run one core operation and shape its outcome for MCP.
    ///
    /// `heimdall-core` is synchronous and filesystem-bound, so the call goes to a
    /// blocking worker instead of stalling the reactor that is also servicing
    /// cancellation and other in-flight requests.
    async fn run<T, F>(&self, vault: Option<String>, operation: F) -> ToolResult<T>
    where
        F: FnOnce(&Vault) -> CoreResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let served = Arc::clone(&self.served);
        let call = move || {
            let vault = served.open(vault.as_deref())?;
            operation(&vault)
        };
        match tokio::task::spawn_blocking(call).await {
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
        name = "read",
        description = "Read a folder or a Markdown note in one of the user's vaults, named by \
                       `vault` (a name from the server instructions, never a path). Omit \
                       `path` to read the vault root. A folder returns `listing`: paths, kinds, sizes, \
                       modification times, and whether each is locked — never content. It is \
                       non-recursive unless `recursive: true` (depth 4 by default, maximum 16), \
                       pages at 50 entries (maximum 200), and returns `next_cursor` to pass \
                       back as `cursor`. A note returns `document`: at most 200 lines by \
                       default (maximum 1000) and 64 KiB (maximum 256 KiB), with \
                       `complete: false` and `next_line` when more remains, and the \
                       `revision` to pass to write. Cannot read hidden folders, the trash, or \
                       files that are not Markdown. Read-only.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn read(
        &self,
        Parameters(request): Parameters<ReadRequest>,
    ) -> Result<Json<ReadResponse>, CallToolResult> {
        let vault = request.vault.clone();
        self.run(vault, move |vault| commands::read(vault, request)).await
    }

    #[tool(
        name = "write",
        description = "MUTATES CONTENT. Create a Markdown note, or replace one completely, in \
                       the vault named by `vault`. \
                       This is a whole-file write, not a patch or an append — send the full \
                       text. To replace a note, pass the `expected_revision` from your latest \
                       read of it; a stale revision fails with REVISION_CONFLICT and nothing \
                       is written. Without a revision it only creates, and fails with \
                       REVISION_CONFLICT if the note exists. The note's folder must already \
                       exist. A locked note, or a new note in a locked folder, fails with \
                       LOCKED: that note is read-only by the user's decision, so do not try \
                       to change it any other way (shell, file tools, scripts, or operating an \
                       app) — tell the user it is locked and where (`locked_at`). Cannot write \
                       hidden folders or non-Markdown files; content is capped at 1 MiB.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn write(
        &self,
        Parameters(request): Parameters<WriteRequest>,
    ) -> Result<Json<WriteResponse>, CallToolResult> {
        let vault = request.vault.clone();
        self.run(vault, move |vault| commands::write(vault, request)).await
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
            .with_instructions(format!("{}\n\n{INSTRUCTIONS}", self.served.describe()))
    }
}

/// Serve over stdio until the client disconnects: the shared vaults, or the
/// one `vault` names.
///
/// Nothing about a vault stops the server starting. A shared vault is opened
/// by each call that needs it, and a fixed one that cannot be opened now is
/// tried again on each call, so the client always lists the tools and the
/// reason for any failure reaches the user through them.
pub fn serve(vault_path: Option<&Utf8Path>) -> CoreResult<()> {
    let server = match vault_path {
        None => {
            let data_dir = appdata::data_dir()?;
            // Housekeeping only: a deleted vault is already never offered
            // (`agents::list` skips it), and this never stops a start.
            let _ = heimdall_core::registry::forget_missing(&data_dir);
            HeimdallServer::shared(data_dir)
        }
        Some(path) => match Vault::open(path) {
            Ok(vault) => {
                // A vault served over MCP is registered too, so the shell can
                // find it from inside; a convenience that never stops a start.
                let _ = heimdall_core::registry::register(&vault);
                HeimdallServer::new(Arc::new(vault))
            }
            Err(error) => {
                eprintln!(
                    "heimdall mcp: the vault cannot be opened ({}: {}); every call will report it",
                    error.code, error.message
                );
                HeimdallServer::serving(Served::Unopened {
                    path: path.to_owned(),
                })
            }
        },
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| {
            Error::internal(format!("could not start the async runtime: {}", err.kind()))
        })?;

    runtime.block_on(async move {
        // Diagnostics go to stderr; stdout carries protocol JSON only (SPEC §12).
        eprintln!("heimdall mcp: serving over stdio, protocol {PROTOCOL_VERSION}");

        let service = server
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
    fn the_surface_is_exactly_the_two_tools() {
        let mut names: Vec<_> = tools().iter().map(|t| t.name.to_string()).collect();
        names.sort();
        assert_eq!(names, ["read", "write"]);
    }

    #[test]
    fn locks_are_not_reachable_over_mcp() {
        // A lock is the user's decision about what an agent may change, so an
        // agent can neither lift one nor set one (SPEC §6, §9). `LockRequest`
        // derives no `JsonSchema`, so a tool for it would not compile; this
        // catches a spelling that dodges the type.
        let names: Vec<_> = tools().iter().map(|t| t.name.to_string()).collect();
        for forbidden in ["lock", "unlock", "set_lock", "lock_path", "unlock_path"] {
            assert!(!names.contains(&forbidden.to_string()), "{forbidden} exposed");
        }
    }

    #[test]
    fn the_desktop_client_commands_are_not_reachable_over_mcp() {
        // Moving, deleting, and indexing a whole vault are things a human does
        // at the keyboard — the same category as `heimdall create` (SPEC §7,
        // §9). They are shell commands the desktop calls; the tool surface
        // above stays at two.
        //
        // This test is a second line of defence, not the first. None of those
        // commands derive `JsonSchema`, so giving one a `#[tool]` would not
        // compile. If this assertion ever starts doing real work, something has
        // been added to the request types that should not have been.
        let names: Vec<_> = tools().iter().map(|t| t.name.to_string()).collect();
        for forbidden in [
            "create_folder",
            "move_path",
            "relink",
            "delete_path",
            "link_graph",
            "create-folder",
            "move-path",
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
    fn the_only_vault_input_is_a_shared_vault_name() {
        // A call may say which shared vault it means, by name; it can never
        // hand the server a place on disk (SPEC §7).
        for tool in tools() {
            let schema = serde_json::to_value(&*tool.input_schema).unwrap();
            let vault = &schema["properties"]["vault"];
            assert_eq!(vault["type"], json!(["string", "null"]), "{}", tool.name);
            let description = vault["description"].as_str().unwrap_or_default();
            assert!(description.contains("name"), "{}: {description}", tool.name);
            assert!(description.contains("never a filesystem path"), "{}", tool.name);

            let text = schema.to_string();
            for forbidden in ["\"root\"", "\"vault_id\"", "\"vault_path\"", "\"directory\""] {
                assert!(!text.contains(forbidden), "{} accepts {forbidden}", tool.name);
            }
            let required = schema["required"].as_array().cloned().unwrap_or_default();
            assert!(!required.contains(&json!("vault")), "{} requires vault", tool.name);
        }
    }

    #[test]
    fn annotations_say_read_only_and_never_open_world() {
        // Clients use these to decide what to confirm. `write` is additive by
        // design — it cannot overwrite without a revision — so it is not
        // marked destructive, and bulk note creation is not met with a prompt
        // per note; locks are what keep a note from changing.
        for tool in tools() {
            let annotations = tool.annotations.clone().expect("annotations");
            assert_eq!(annotations.open_world_hint, Some(false), "{}", tool.name);
            match tool.name.as_ref() {
                "read" => assert_eq!(annotations.read_only_hint, Some(true)),
                "write" => {
                    assert_eq!(annotations.read_only_hint, Some(false));
                    assert_eq!(annotations.destructive_hint, Some(false));
                }
                other => panic!("unexpected tool {other}"),
            }
        }
    }

    #[test]
    fn a_locked_refusal_is_spelled_out_in_the_write_description_too() {
        // Some clients never pass `instructions` to the model, so the one tool
        // that can be refused says how to take the refusal.
        let write = tools().into_iter().find(|t| t.name == "write").unwrap();
        let description = write.description.as_deref().unwrap_or_default();
        assert!(description.contains("LOCKED"));
        assert!(description.contains("do not try"), "{description}");
        assert!(description.contains("`locked_at`"));
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
            let mutates = tool.name == "write";
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
        for needle in ["`read`", "`write`", "locked", "LOCKED", "`locked_at`"] {
            assert!(instructions.contains(needle), "instructions never mention {needle}");
        }
        // The guardrail names the routes an agent might reach for instead.
        for route in ["shell", "file-editing", "scripts", "Heimdall app", "computer use", "accessibility"] {
            assert!(instructions.contains(route), "instructions never rule out {route}");
        }
        // The vault it serves is named, so an agent with several servers or
        // vaults knows which this is.
        let name = root.file_name().unwrap();
        assert!(instructions.contains(&format!("\"{name}\"")), "{instructions}");
        for tool in ["`lock`", "`unlock`"] {
            assert!(!instructions.contains(tool), "instructions offer {tool}");
        }
    }

    #[test]
    fn a_shared_server_names_every_shared_vault_and_never_a_path() {
        let data = tempfile::tempdir().unwrap();
        let data = camino::Utf8PathBuf::from_path_buf(data.path().to_path_buf()).unwrap();
        let parent = tempfile::tempdir().unwrap();
        let mut roots = Vec::new();
        for folder in ["Work", "Personal"] {
            let root = parent.path().join(folder);
            std::fs::create_dir(&root).unwrap();
            let root = camino::Utf8PathBuf::from_path_buf(root).unwrap();
            let vault = Vault::open_with_data_dir(&root, &data).unwrap();
            agents::share(&vault, None).unwrap();
            roots.push(root);
        }

        let info = HeimdallServer::shared(data.clone()).get_info();
        let instructions = info.instructions.unwrap();
        assert!(instructions.contains("\"Personal\", \"Work\""), "{instructions}");
        assert!(instructions.contains("Pass `vault`"), "{instructions}");
        for root in roots {
            assert!(!instructions.contains(root.as_str()), "a path leaked: {instructions}");
        }
    }

    #[test]
    fn a_shared_server_with_nothing_shared_still_starts_and_says_so() {
        let data = tempfile::tempdir().unwrap();
        let data = camino::Utf8PathBuf::from_path_buf(data.path().to_path_buf()).unwrap();
        let instructions = HeimdallServer::shared(data).get_info().instructions.unwrap();
        assert!(instructions.contains("No vault was shared"), "{instructions}");
    }

    #[test]
    fn a_domain_failure_is_structured_and_flagged_without_leaking_content() {
        let error = Error::revision_conflict("stale")
            .with_detail("path", "projects/plan.md")
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
