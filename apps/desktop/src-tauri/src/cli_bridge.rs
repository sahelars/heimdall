//! The bridge to the bundled `heimdall` CLI (SPEC §15).
//!
//! This is the desktop's only route to the domain. It never links
//! `heimdall-core`, never searches `PATH`, and never runs a shell: it spawns one
//! known executable with an argument vector built from an allowlist, hands
//! Markdown over stdin, and parses the CLI's versioned JSON envelope.
//!
//! React reaches this through two Tauri commands and cannot name an executable,
//! pass a raw command string, or invent a flag.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The product's executable name (SPEC §19). On a bundled app the sidecar sits
/// beside the main binary under exactly this name.
const SIDECAR_NAME: &str = "heimdall";

/// How long any single CLI invocation may take before it is killed.
///
/// Every command but one is a bounded read or a single write, so 30 seconds is
/// already generous. `link-graph` is the exception and gets its own budget.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a whole-vault operation may take.
///
/// `link-graph` reads every note in the vault (SPEC §8), and `relink` reads it
/// and then writes the notes that linked at what moved. On a cold disk a large
/// vault can approach the ordinary ceiling, and killing either one — a graph
/// the user asked for, or a rename's follow-up half done — is worse than
/// waiting for it.
const GRAPH_TIMEOUT: Duration = Duration::from_secs(120);

/// How long the MCP health check waits for a handshake response.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// The most stderr kept for diagnostics. Bounded so a chatty failure cannot
/// exhaust memory or push note content into a log.
const STDERR_LIMIT: usize = 8 * 1024;

/// The most stdout accepted from one call.
///
/// Sized by `link-graph`, which is by far the largest response the CLI produces
/// and the only one whose size grows with the vault: a 5,000-node graph at ten
/// links a note is roughly 4 MB once the envelope is pretty-printed. The bounded
/// reads it dwarfs are capped at 256 KiB each.
const STDOUT_LIMIT: usize = 16 * 1024 * 1024;

/// Every CLI subcommand the desktop may run, and the argument keys each
/// accepts. A request naming anything else is refused before a process starts,
/// so React cannot reach a subcommand this list does not mention.
///
/// `mcp` is absent deliberately: the server is long-running and is started by
/// the client, not by a desktop request. The health check spawns it through its
/// own dedicated path.
const ALLOWED: &[(&str, &[&str])] = &[
    ("create", &["name", "root"]),
    // The four verbs the MCP server also offers (SPEC §9).
    (
        "read",
        &[
            "vault",
            "path",
            "start-line",
            "max-lines",
            "max-total-bytes",
            "recursive",
            "max-depth",
            "cursor",
            "limit",
        ],
    ),
    ("write", &["vault", "path", "expected-revision", "create"]),
    ("lock", &["vault", "path"]),
    ("unlock", &["vault", "path"]),
    // Client operations: the desktop edits the vault, and none of these is an
    // MCP tool (SPEC §9, §15).
    ("create-folder", &["vault", "path"]),
    ("move-path", &["vault", "from", "to"]),
    ("relink", &["vault", "from", "to", "dry-run"]),
    ("delete-path", &["vault", "path", "expected-revision"]),
    ("link-graph", &["vault", "max-depth"]),
];

/// The subcommands whose `path` is positional rather than a flag.
///
/// It is passed after `--`, so a note whose name begins with a dash is still a
/// path and never an option.
const POSITIONAL_PATH: &[&str] = &["read", "write", "lock", "unlock"];

/// The subcommands that read or write the whole vault, and so need the longer
/// budget rather than the ordinary one.
const WHOLE_VAULT: &[&str] = &["link-graph", "relink"];

/// The subcommands that change the vault.
///
/// Only these need exclusive access. Everything else takes a shared guard, so a
/// whole-vault `link-graph` — seconds of work on a large vault — never blocks
/// the editor's autosave behind it.
const MUTATES: &[&str] = &[
    "create",
    "write",
    "lock",
    "unlock",
    "create-folder",
    "move-path",
    "relink",
    "delete-path",
];

/// What the CLI returned, in the shape React consumes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliResponse {
    pub ok: bool,
    /// The `data` payload on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    /// The structured `error` on an expected domain failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<DomainError>,
    pub schema_version: Option<u64>,
    pub exit_code: Option<i32>,
    /// Bounded stderr, kept for the Diagnostics screen.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub details: Value,
}

impl DomainError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            details: Value::Object(Default::default()),
        }
    }
}

impl CliResponse {
    fn failed(error: DomainError, stderr: String, exit_code: Option<i32>) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(error),
            schema_version: None,
            exit_code,
            stderr,
        }
    }
}

/// Which binary the desktop is using, and what it reports about itself.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliStatus {
    /// Absolute path of the bundled sidecar this app will run.
    pub path: String,
    /// Whether that file is actually present and executable.
    pub available: bool,
    /// Whether that path belongs to a build tree rather than a shipped app.
    pub development_build: bool,
    pub cli_version: Option<String>,
    pub core_version: Option<String>,
    pub mcp_protocol_version: Option<String>,
    pub output_schema_version: Option<u64>,
    /// Set when the sidecar is missing or would not answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<DomainError>,
}

/// Serializes writes aimed at the same vault.
///
/// Two windows-worth of user actions racing on one vault would otherwise meet
/// only at the CLI's file lock, where the loser surfaces as a revision conflict
/// the user did not cause.
///
/// Reads take the shared side. They cannot corrupt anything, and holding them
/// exclusively would put every read in the application behind the slowest one —
/// which, now that `link-graph` exists, means the editor freezing while the
/// graph rebuilds.
fn write_locks() -> &'static Mutex<BTreeMap<String, &'static RwLock<()>>> {
    static LOCKS: OnceLock<Mutex<BTreeMap<String, &'static RwLock<()>>>> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn vault_lock(vault: &str) -> &'static RwLock<()> {
    let mut locks = write_locks().lock().expect("write lock registry");
    locks
        .entry(vault.to_string())
        .or_insert_with(|| Box::leak(Box::new(RwLock::new(()))))
}

/// Whether a subcommand needs exclusive access to its vault.
fn mutates(command: &str) -> bool {
    MUTATES.contains(&command)
}

/// Where the sidecar this application runs actually came from.
///
/// [`sidecar_path`] cannot answer this on its own. Under `tauri dev` the CLI is
/// staged beside the development executable, so the "beside `current_exe`"
/// branch matches there exactly as it does inside a shipped bundle, and the two
/// are indistinguishable by path — short of matching on the string `target`,
/// which is a directory name a user is free to choose.
///
/// It matters for one decision only: what may be written into a client's own
/// configuration file (SPEC §15). Running the development sidecar is what
/// development is for, so nothing else consults this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarOrigin {
    /// Bundled inside an installed application; its path outlives this session.
    Shipped,
    /// A build artifact, which a rebuild, a rename, or `cargo clean` removes.
    Development,
}

impl SidecarOrigin {
    pub fn is_development(self) -> bool {
        self == SidecarOrigin::Development
    }
}

/// Which of the two this build is.
///
/// `tauri::is_dev()` is `!cfg!(feature = "custom-protocol")`, and `tauri build`
/// compiles with `--features tauri/custom-protocol` while `tauri dev` does not.
/// So this is the same answer the bundler had — a compile-time fact, not a
/// guess made from the filesystem.
pub fn sidecar_origin() -> SidecarOrigin {
    if tauri::is_dev() {
        SidecarOrigin::Development
    } else {
        SidecarOrigin::Shipped
    }
}

/// Resolve the bundled sidecar. `PATH` is never consulted (SPEC §16).
///
/// A bundled app keeps the sidecar beside its main executable under the product
/// name; a development build keeps it in `src-tauri/binaries/` under Tauri's
/// target-triple naming. Both are absolute paths to this app's own binary, so a
/// separately installed `heimdall` is never picked up by accident.
pub fn sidecar_path() -> PathBuf {
    if let Some(beside) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(SIDECAR_NAME)))
        .filter(|candidate| candidate.is_file())
    {
        return beside;
    }
    development_sidecar()
}

/// Where `tauri dev` leaves the staged sidecar.
fn development_sidecar() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("binaries")
        .join(format!("{SIDECAR_NAME}-{}", target_triple()))
}

/// The triple Tauri suffixes `externalBin` artifacts with.
fn target_triple() -> String {
    // Recorded at build time by tauri-build; falling back keeps unit tests and
    // `cargo test` working without the build script's environment.
    option_env!("TAURI_ENV_TARGET_TRIPLE")
        .map(str::to_string)
        .unwrap_or_else(|| std::env::var("TARGET").unwrap_or_else(|_| host_triple().to_string()))
}

const fn host_triple() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64"),
    )))]
    {
        "unknown"
    }
}

/// Turn an allowlisted command and its request into an argument vector.
///
/// Values become separate `argv` entries, never interpolated into a string, so
/// a vault path containing spaces, quotes, or a semicolon is just a path.
fn build_args(command: &str, request: &Value) -> Result<Vec<String>, DomainError> {
    let Some((_, allowed_keys)) = ALLOWED.iter().find(|(name, _)| *name == command) else {
        return Err(DomainError::new(
            "INVALID_INPUT",
            format!("\"{command}\" is not a command this application can run"),
        ));
    };

    let mut args = vec![command.to_string()];

    // `create` takes the vault name positionally.
    if command == "create" {
        let name = request
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| DomainError::new("INVALID_INPUT", "create requires a vault name"))?;
        args.push(name.to_string());
    }

    let Some(fields) = request.as_object() else {
        return Err(DomainError::new(
            "INVALID_INPUT",
            "the request must be a JSON object",
        ));
    };

    let positional_path = POSITIONAL_PATH.contains(&command);
    let mut path = None;

    // Sorted so one request always produces one argument vector, which keeps
    // failures reproducible from a diagnostics report.
    for (key, value) in fields.iter().collect::<BTreeMap<_, _>>() {
        if command == "create" && key.as_str() == "name" {
            continue;
        }
        if !allowed_keys.contains(&key.as_str()) {
            return Err(DomainError::new(
                "INVALID_INPUT",
                format!("\"{key}\" is not an argument of \"{command}\""),
            ));
        }
        if positional_path && key.as_str() == "path" {
            path = match value {
                Value::Null => None,
                Value::String(text) => Some(text.clone()),
                _ => {
                    return Err(DomainError::new(
                        "INVALID_INPUT",
                        format!("\"path\" of \"{command}\" must be a string"),
                    ))
                }
            };
            continue;
        }
        push_argument(&mut args, key, value)?;
    }
    if let Some(path) = path {
        args.push("--".to_string());
        args.push(path);
    }
    Ok(args)
}

fn push_argument(args: &mut Vec<String>, key: &str, value: &Value) -> Result<(), DomainError> {
    let flag = format!("--{key}");
    match value {
        Value::Null => {}
        // A flag with no value; `false` means "do not pass it at all".
        Value::Bool(true) => args.push(flag),
        Value::Bool(false) => {}
        Value::String(text) => {
            args.push(flag);
            args.push(text.clone());
        }
        Value::Number(number) => {
            args.push(flag);
            args.push(number.to_string());
        }
        // No command takes a repeated or structured argument.
        Value::Array(_) | Value::Object(_) => {
            return Err(DomainError::new(
                "INVALID_INPUT",
                format!("\"{key}\" must be a single value"),
            ))
        }
    }
    Ok(())
}

/// Run the bundled CLI once and return its parsed envelope.
pub fn run(command: &str, request: &Value, stdin: Option<&str>) -> CliResponse {
    let args = match build_args(command, request) {
        Ok(args) => args,
        Err(error) => return CliResponse::failed(error, String::new(), None),
    };

    // Hold the vault's lock for the whole call so two writes to one vault cannot
    // interleave into a conflict the user never caused. A read only needs the
    // shared side, so reads never queue behind each other.
    let timeout = if WHOLE_VAULT.contains(&command) {
        GRAPH_TIMEOUT
    } else {
        CALL_TIMEOUT
    };
    let call = || execute(&sidecar_path(), &args, stdin, timeout);

    match request.get("vault").and_then(Value::as_str).map(vault_lock) {
        Some(lock) if mutates(command) => {
            let _guard = lock.write().expect("vault write lock");
            call()
        }
        Some(lock) => {
            let _guard = lock.read().expect("vault read lock");
            call()
        }
        // `create` names a root and a folder rather than an existing vault, so
        // there is nothing yet to serialise against.
        None => call(),
    }
}

fn execute(
    binary: &Path,
    args: &[String],
    stdin: Option<&str>,
    timeout: Duration,
) -> CliResponse {
    if !binary.is_file() {
        return CliResponse::failed(
            DomainError::new(
                "IO_ERROR",
                "the bundled heimdall command line tool is missing from this application",
            ),
            String::new(),
            None,
        );
    }

    // No shell: an argument vector goes straight to exec, so nothing in a path
    // or a filename is ever interpreted.
    let mut child = match Command::new(binary)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return CliResponse::failed(
                DomainError::new("IO_ERROR", format!("could not start the tool: {}", err.kind())),
                String::new(),
                None,
            )
        }
    };

    // Markdown always arrives on stdin, never as an argument (SPEC §11).
    if let Some(mut pipe) = child.stdin.take() {
        let _ = pipe.write_all(stdin.unwrap_or_default().as_bytes());
    }

    let stdout = take_bounded(child.stdout.take(), STDOUT_LIMIT);
    let stderr = take_bounded(child.stderr.take(), STDERR_LIMIT);

    let exit_code = match wait_bounded(&mut child, timeout) {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return CliResponse::failed(
                DomainError::new(
                    "IO_ERROR",
                    format!(
                        "the tool did not finish within {} seconds and was stopped",
                        timeout.as_secs()
                    ),
                ),
                stderr,
                None,
            );
        }
    };

    // Output that filled the buffer was almost certainly cut off mid-JSON.
    // Saying so beats letting `parse_envelope` report it as unreadable output,
    // which tells the user nothing about the one thing they could act on.
    if stdout.len() >= STDOUT_LIMIT {
        return CliResponse::failed(
            DomainError::new(
                "LIMIT_EXCEEDED",
                format!(
                    "the tool produced more than {} MB of output; narrow the request",
                    STDOUT_LIMIT / (1024 * 1024)
                ),
            ),
            stderr,
            Some(exit_code),
        );
    }

    parse_envelope(&stdout, stderr, exit_code)
}

/// Read a stream to its limit, discarding anything beyond.
fn take_bounded<R: Read + Send + 'static>(stream: Option<R>, limit: usize) -> String {
    let Some(stream) = stream else {
        return String::new();
    };
    let mut buffer = Vec::new();
    let mut reader = stream.take(limit as u64);
    let _ = reader.read_to_end(&mut buffer);
    String::from_utf8_lossy(&buffer).trim().to_string()
}

/// Wait for the child, giving up after `limit`.
fn wait_bounded(child: &mut Child, limit: Duration) -> Option<i32> {
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.code().unwrap_or(-1)),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => return None,
            Err(_) => return Some(-1),
        }
    }
}

/// Turn the CLI's versioned envelope into typed data (SPEC §11).
fn parse_envelope(stdout: &str, stderr: String, exit_code: i32) -> CliResponse {
    let Ok(envelope) = serde_json::from_str::<Value>(stdout) else {
        return CliResponse::failed(
            DomainError::new(
                "INTERNAL_ERROR",
                "the tool returned output this application could not read",
            ),
            stderr,
            Some(exit_code),
        );
    };

    let schema_version = envelope
        .get("meta")
        .and_then(|meta| meta.get("schema_version"))
        .and_then(Value::as_u64);

    if envelope.get("ok").and_then(Value::as_bool) == Some(true) {
        return CliResponse {
            ok: true,
            data: envelope.get("data").cloned(),
            error: None,
            schema_version,
            exit_code: Some(exit_code),
            stderr,
        };
    }

    let error = envelope
        .get("error")
        .and_then(|error| serde_json::from_value::<DomainError>(error.clone()).ok())
        .unwrap_or_else(|| {
            DomainError::new("INTERNAL_ERROR", "the tool reported a failure without a code")
        });

    CliResponse {
        ok: false,
        data: None,
        error: Some(error),
        schema_version,
        exit_code: Some(exit_code),
        stderr,
    }
}

/// Report which binary is in use and what it says about itself.
pub fn status() -> CliStatus {
    let path = sidecar_path();
    let display = path.to_string_lossy().to_string();

    if !path.is_file() {
        return CliStatus {
            path: display,
            available: false,
            development_build: sidecar_origin().is_development(),
            cli_version: None,
            core_version: None,
            mcp_protocol_version: None,
            output_schema_version: None,
            error: Some(DomainError::new(
                "IO_ERROR",
                "the bundled heimdall command line tool is missing from this application",
            )),
        };
    }

    let response = execute(
        &path,
        &["--version".to_string(), "--json".to_string()],
        None,
        CALL_TIMEOUT,
    );
    let data = response.data.unwrap_or(Value::Null);
    let text = |key: &str| data.get(key).and_then(Value::as_str).map(str::to_string);

    CliStatus {
        path: display,
        available: response.ok,
        development_build: sidecar_origin().is_development(),
        cli_version: text("cli_version"),
        core_version: text("core_version"),
        mcp_protocol_version: text("mcp_protocol_version"),
        output_schema_version: data
            .get("output_schema_version")
            .and_then(Value::as_u64),
        error: response.error,
    }
}

/// What a one-click health check found.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheck {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    pub tool_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<DomainError>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub stderr: String,
}

impl HealthCheck {
    fn failed(code: &str, message: impl Into<String>, stderr: String) -> Self {
        Self {
            ok: false,
            server_name: None,
            server_version: None,
            protocol_version: None,
            instructions: None,
            tool_count: None,
            error: Some(DomainError::new(code, message)),
            stderr,
        }
    }
}

/// Launch the MCP server and complete a real handshake against it (SPEC §15).
///
/// This answers the question a user actually has — "will my AI client be able to
/// talk to this?" — by doing what that client does, rather than by checking that
/// a file exists.
pub fn health_check(vault: &str) -> HealthCheck {
    let binary = sidecar_path();
    if !binary.is_file() {
        return HealthCheck::failed(
            "IO_ERROR",
            "the bundled heimdall command line tool is missing from this application",
            String::new(),
        );
    }

    let mut child = match Command::new(&binary)
        .args(["mcp", "--vault", vault])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return HealthCheck::failed(
                "IO_ERROR",
                format!("could not start the server: {}", err.kind()),
                String::new(),
            )
        }
    };

    let outcome = handshake(&mut child);
    let _ = child.kill();
    let stderr = take_bounded(child.stderr.take(), STDERR_LIMIT);
    let _ = child.wait();

    match outcome {
        Ok(mut check) => {
            check.stderr = stderr;
            check
        }
        Err(error) => HealthCheck {
            ok: false,
            server_name: None,
            server_version: None,
            protocol_version: None,
            instructions: None,
            tool_count: None,
            error: Some(error),
            stderr,
        },
    }
}

fn handshake(child: &mut Child) -> Result<HealthCheck, DomainError> {
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| DomainError::new("IO_ERROR", "the server accepted no input"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| DomainError::new("IO_ERROR", "the server produced no output"))?;

    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": { "name": "heimdall-desktop", "version": env!("CARGO_PKG_VERSION") }
        }
    });
    writeln!(stdin, "{request}")
        .and_then(|()| stdin.flush())
        .map_err(|err| {
            DomainError::new("IO_ERROR", format!("could not reach the server: {}", err.kind()))
        })?;

    // Read on a worker so a server that never answers cannot wedge the app.
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let read = BufReader::new(stdout).read_line(&mut line);
        let _ = sender.send(read.map(|_| line));
    });

    let line = match receiver.recv_timeout(HANDSHAKE_TIMEOUT) {
        Ok(Ok(line)) if !line.trim().is_empty() => line,
        Ok(Ok(_)) => {
            return Err(DomainError::new(
                "IO_ERROR",
                "the server closed the connection without answering",
            ))
        }
        Ok(Err(err)) => {
            return Err(DomainError::new(
                "IO_ERROR",
                format!("could not read from the server: {}", err.kind()),
            ))
        }
        Err(_) => {
            return Err(DomainError::new(
                "IO_ERROR",
                format!(
                    "the server did not answer within {} seconds",
                    HANDSHAKE_TIMEOUT.as_secs()
                ),
            ))
        }
    };

    let message: Value = serde_json::from_str(line.trim()).map_err(|_| {
        DomainError::new("INTERNAL_ERROR", "the server sent a reply this application could not read")
    })?;

    if let Some(error) = message.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("the server refused the handshake");
        return Err(DomainError::new("INTERNAL_ERROR", message));
    }

    let result = message
        .get("result")
        .ok_or_else(|| DomainError::new("INTERNAL_ERROR", "the server sent no handshake result"))?;
    let text = |path: &[&str]| -> Option<String> {
        let mut node = result;
        for key in path {
            node = node.get(key)?;
        }
        node.as_str().map(str::to_string)
    };

    Ok(HealthCheck {
        ok: true,
        server_name: text(&["serverInfo", "name"]),
        server_version: text(&["serverInfo", "version"]),
        protocol_version: text(&["protocolVersion"]),
        instructions: text(&["instructions"]),
        tool_count: None,
        error: None,
        stderr: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(value: Value) -> Value {
        value
    }

    #[test]
    fn only_allowlisted_commands_can_run() {
        for command in ["create", "read", "write", "lock", "unlock"] {
            assert!(build_args(command, &json!({})).is_ok() || command == "create");
        }
        for command in [
            "mcp",
            "rm",
            "sh",
            "bash",
            "",
            "read_file",
            "list-documents",
            "write-document",
            "../heimdall",
        ] {
            let error = build_args(command, &json!({})).unwrap_err();
            assert_eq!(error.code, "INVALID_INPUT", "accepted {command:?}");
        }
    }

    #[test]
    fn every_client_operation_is_reachable_and_carries_its_own_arguments() {
        // The desktop edits the vault through these; none of them is an MCP
        // tool (SPEC §9, §15).
        let args = build_args(
            "create-folder",
            &json!({ "vault": "/v", "path": "a b" }),
        )
        .unwrap();
        // Keys are sorted and the path is one argv entry rather than something
        // a shell could split.
        assert_eq!(args, ["create-folder", "--path", "a b", "--vault", "/v"]);

        for command in [
            "create-folder",
            "move-path",
            "relink",
            "delete-path",
            "link-graph",
        ] {
            assert!(
                build_args(command, &json!({ "vault": "/v" })).is_ok(),
                "{command} is unreachable"
            );
        }
    }

    #[test]
    fn a_client_operation_cannot_borrow_another_commands_arguments() {
        let error = build_args("link-graph", &json!({ "path": "a.md" })).unwrap_err();
        assert_eq!(error.code, "INVALID_INPUT");

        let error = build_args("lock", &json!({ "expected-revision": "x" })).unwrap_err();
        assert_eq!(error.code, "INVALID_INPUT");

        let error = build_args("delete-path", &json!({ "create": true })).unwrap_err();
        assert_eq!(error.code, "INVALID_INPUT");
    }

    #[test]
    fn every_mutating_command_is_one_the_bridge_can_actually_run() {
        // Two lists that must not drift: a name in MUTATES that ALLOWED does not
        // have is dead weight, and a write missing from MUTATES would quietly
        // take the shared guard and stop serialising.
        for command in MUTATES {
            assert!(
                ALLOWED.iter().any(|(name, _)| name == command),
                "{command} mutates but cannot be run"
            );
            assert!(mutates(command));
        }
        // Reads take the shared side, which is what keeps a whole-vault graph
        // from blocking the editor.
        for command in ["read", "link-graph"] {
            assert!(!mutates(command), "{command} should not need exclusive access");
        }
    }

    #[test]
    fn a_whole_vault_command_gets_the_longer_budget() {
        // `relink` reads every note and then writes the ones that linked at what
        // moved. Killed at the ordinary 30 seconds it would leave a large vault
        // half rewritten, which is the one outcome worse than not starting.
        for command in ["link-graph", "relink"] {
            assert!(WHOLE_VAULT.contains(&command), "{command} needs the graph budget");
            assert!(
                ALLOWED.iter().any(|(name, _)| *name == command),
                "{command} cannot be run at all"
            );
        }
        assert!(!WHOLE_VAULT.contains(&"write"));
    }

    #[test]
    fn relink_takes_the_exclusive_guard() {
        // It writes, so it must serialise against autosave rather than run
        // beside it — unlike `link-graph`, which only reads.
        assert!(mutates("relink"));
        assert_eq!(
            build_args(
                "relink",
                &json!({ "vault": "/v", "from": "a.md", "to": "b.md", "dry-run": true }),
            )
            .unwrap(),
            ["relink", "--dry-run", "--from", "a.md", "--to", "b.md", "--vault", "/v"]
        );
        assert_eq!(
            build_args("relink", &json!({ "vault": "/v", "path": "a.md" }))
                .unwrap_err()
                .code,
            "INVALID_INPUT"
        );
    }

    #[test]
    fn only_allowlisted_arguments_can_be_passed() {
        let error = build_args("read", &json!({ "exec": "/bin/sh" })).unwrap_err();
        assert_eq!(error.code, "INVALID_INPUT");

        // An argument that belongs to a different subcommand is still refused.
        let error = build_args("read", &json!({ "from": "a.md" })).unwrap_err();
        assert_eq!(error.code, "INVALID_INPUT");
    }

    #[test]
    fn values_become_separate_argv_entries_and_are_never_interpolated() {
        let args = build_args(
            "create-folder",
            &request(json!({ "vault": "/Users/n/My Vault; rm -rf ~", "path": "projects" })),
        )
        .unwrap();

        assert_eq!(
            args,
            vec![
                "create-folder",
                "--path",
                "projects",
                "--vault",
                "/Users/n/My Vault; rm -rf ~",
            ]
        );
        // The hostile-looking path survives intact as one argument, because it
        // is data rather than something a shell will parse.
        assert!(args.contains(&"/Users/n/My Vault; rm -rf ~".to_string()));
    }

    #[test]
    fn create_passes_its_name_positionally() {
        let args = build_args("create", &json!({ "name": "demo", "root": "/tmp" })).unwrap();
        assert_eq!(args, vec!["create", "demo", "--root", "/tmp"]);
    }

    #[test]
    fn the_four_verbs_take_their_path_positionally_after_the_options() {
        // After `--`, so a note named like a flag is still only a path.
        let args = build_args(
            "write",
            &json!({ "vault": "/v", "path": "--vault.md", "expected-revision": "blake3:ab" }),
        )
        .unwrap();
        assert_eq!(
            args,
            ["write", "--expected-revision", "blake3:ab", "--vault", "/v", "--", "--vault.md"]
        );

        // No path means the vault root, and nothing follows the options.
        assert_eq!(
            build_args("lock", &json!({ "vault": "/v" })).unwrap(),
            ["lock", "--vault", "/v"]
        );
        assert_eq!(
            build_args("read", &json!({ "vault": "/v", "path": null })).unwrap(),
            ["read", "--vault", "/v"]
        );
        assert_eq!(
            build_args("unlock", &json!({ "path": 7 })).unwrap_err().code,
            "INVALID_INPUT"
        );
    }

    #[test]
    fn booleans_become_bare_flags_and_false_is_omitted() {
        let args = build_args("read", &json!({ "recursive": true })).unwrap();
        assert_eq!(args, vec!["read", "--recursive"]);

        let args = build_args("read", &json!({ "recursive": false })).unwrap();
        assert_eq!(args, vec!["read"]);
    }

    #[test]
    fn numbers_are_rendered_without_a_shell_seeing_them() {
        let args = build_args("read", &json!({ "limit": 50 })).unwrap();
        assert_eq!(args, vec!["read", "--limit", "50"]);
    }

    #[test]
    fn nested_and_repeated_values_are_refused() {
        let error = build_args("read", &json!({ "limit": { "$gt": 1 } })).unwrap_err();
        assert_eq!(error.code, "INVALID_INPUT");
        let error = build_args("read", &json!({ "cursor": ["a", "b"] })).unwrap_err();
        assert_eq!(error.code, "INVALID_INPUT");
    }

    #[test]
    fn the_mcp_server_is_not_reachable_as_an_ordinary_command() {
        // Starting a long-running server is the client's job, and the health
        // check has its own path; React must not be able to spawn one.
        assert!(build_args("mcp", &json!({ "vault": "/v" })).is_err());
        assert!(!ALLOWED.iter().any(|(name, _)| *name == "mcp"));
    }

    #[test]
    fn the_sidecar_is_always_an_absolute_path_and_never_a_bare_name() {
        // A bare name is what a `PATH` lookup would use; this must never be one.
        let path = sidecar_path();
        assert!(path.is_absolute(), "{path:?}");
        assert!(path.parent().is_some());
        assert_eq!(
            path.file_name().unwrap().to_string_lossy().to_string(),
            development_sidecar()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string(),
        );
    }

    #[test]
    fn the_development_sidecar_carries_tauris_target_triple_suffix() {
        let name = development_sidecar()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(name.starts_with("heimdall-"), "{name}");
        assert!(name.contains(&target_triple()), "{name}");
    }

    #[test]
    fn a_missing_sidecar_is_reported_rather_than_falling_back_to_path() {
        let response = execute(
            Path::new("/nonexistent/heimdall"),
            &["--version".to_string()],
            None,
            CALL_TIMEOUT,
        );
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "IO_ERROR");
    }

    #[test]
    fn a_success_envelope_becomes_typed_data() {
        let response = parse_envelope(
            r#"{"ok":true,"data":{"mode":"scaffolded"},"meta":{"schema_version":1}}"#,
            String::new(),
            0,
        );
        assert!(response.ok);
        assert_eq!(response.data.unwrap()["mode"], "scaffolded");
        assert_eq!(response.schema_version, Some(1));
    }

    #[test]
    fn an_error_envelope_keeps_its_code_and_details() {
        let response = parse_envelope(
            r#"{"ok":false,"error":{"code":"REVISION_CONFLICT","message":"stale","details":{"current_revision":"blake3:abc"}},"meta":{"schema_version":1}}"#,
            "some warning".to_string(),
            1,
        );
        assert!(!response.ok);
        let error = response.error.unwrap();
        assert_eq!(error.code, "REVISION_CONFLICT");
        assert_eq!(error.details["current_revision"], "blake3:abc");
        assert_eq!(response.stderr, "some warning");
        assert_eq!(response.exit_code, Some(1));
    }

    #[test]
    fn unreadable_output_is_reported_without_panicking() {
        let response = parse_envelope("not json at all", String::new(), 0);
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "INTERNAL_ERROR");
    }

    #[test]
    fn a_test_build_is_a_development_one() {
        // `cargo test` compiles without `tauri/custom-protocol`, exactly as
        // `tauri dev` does. This is here so that hardcoding either variant —
        // which would leave the client-configuration guard silently dead, in
        // one direction or the other — fails rather than ships.
        assert_eq!(sidecar_origin(), SidecarOrigin::Development);
        assert!(sidecar_origin().is_development());
    }

    #[test]
    fn stderr_is_captured_but_bounded() {
        let noisy = std::io::Cursor::new(vec![b'x'; STDERR_LIMIT * 4]);
        let captured = take_bounded(Some(noisy), STDERR_LIMIT);
        assert_eq!(captured.len(), STDERR_LIMIT);
    }
}
