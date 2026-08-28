//! The direct shell command surface (SPEC §9, §11).
//!
//! Subcommands are kebab-case and mirror the core operations one-to-one, so the
//! MCP adapter added in Phase 2 can be checked against this surface for
//! equivalent domain outcomes.

use std::io::Read;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use serde_json::json;
use heimdall_core::commands::{self, EntryKind};
use heimdall_core::{limits, Error, Result, Revision, Vault};

use crate::{envelope, mcp};

#[derive(Debug, Parser)]
#[command(
    name = "heimdall",
    about = "An intent-aware layer over Markdown vaults.",
    long_about = "Heimdall gives AI clients constrained, purpose-aware operations over a \
                  Markdown vault instead of unrestricted filesystem access.\n\nOrdinary notes \
                  live outside aios/; protected memories and entries live inside it. Every \
                  command prints a JSON envelope on stdout.",
    disable_version_flag = true
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Print version and compatibility information.
    #[arg(long, global = true)]
    version: bool,

    /// Force JSON output. Shell commands always emit JSON; this also applies to
    /// --version.
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the MCP server for one vault over stdio.
    Mcp {
        /// Path to the vault this server exposes. Required: the vault is fixed
        /// by configuration and never by a tool call.
        #[arg(long, value_name = "PATH")]
        vault: Utf8PathBuf,
    },

    /// Create a new templated vault, or add the managed structure to an existing one.
    Create {
        /// One folder name, not a path.
        name: String,
        /// Where to create it. Defaults to the working directory.
        #[arg(long)]
        root: Option<Utf8PathBuf>,
    },

    /// List ordinary Markdown notes. Never reaches aios/.
    ListDocuments {
        #[command(flatten)]
        vault: VaultArg,
        /// Directory to list, relative to the vault root.
        #[arg(long)]
        path: Option<String>,
        /// Descend into subdirectories.
        #[arg(long)]
        recursive: bool,
        /// Maximum depth for a recursive listing (default 4, maximum 16).
        #[arg(long)]
        max_depth: Option<u32>,
        /// The next_cursor from a previous page.
        #[arg(long)]
        cursor: Option<String>,
        /// Page size (default 50, maximum 200).
        #[arg(long)]
        limit: Option<u32>,
    },

    /// Read bounded ranges of selected ordinary notes.
    ReadDocuments {
        #[command(flatten)]
        vault: VaultArg,
        /// A document to read. Repeat for up to 10 documents.
        #[arg(long = "doc", value_name = "PATH")]
        doc: Vec<String>,
        /// First line to return, applied to every --doc.
        #[arg(long)]
        start_line: Option<u32>,
        /// Lines to return per document (default 200, maximum 1000).
        #[arg(long)]
        max_lines: Option<u32>,
        /// Total content budget in bytes (default 65536, maximum 262144).
        #[arg(long)]
        max_total_bytes: Option<u32>,
        /// Read the full JSON request from stdin instead of using --doc.
        /// Use this for per-document line ranges.
        #[arg(long, conflicts_with_all = ["doc", "start_line", "max_lines", "max_total_bytes"])]
        request: bool,
    },

    /// List the main memory and extended memories.
    ListMemories {
        #[command(flatten)]
        vault: VaultArg,
        #[arg(long)]
        limit: Option<u32>,
    },

    /// Read the main memory, or one extended memory.
    ReadMemory {
        #[command(flatten)]
        vault: VaultArg,
        /// An extended memory filename. Omit to read the main memory.
        #[arg(long, value_name = "FILENAME")]
        extended: Option<String>,
        #[arg(long)]
        start_line: Option<u32>,
        #[arg(long)]
        max_lines: Option<u32>,
    },

    /// Replace the main memory or one extended memory. Content comes from stdin.
    WriteMemory {
        #[command(flatten)]
        vault: VaultArg,
        /// An extended memory filename. Omit to write the main memory.
        #[arg(long, value_name = "FILENAME")]
        extended: Option<String>,
        /// The revision from the latest read of this file.
        #[arg(long, value_name = "REVISION")]
        expected_revision: Option<String>,
        /// Create a new extended memory that does not exist yet.
        #[arg(long, conflicts_with = "expected_revision")]
        create: bool,
    },

    /// List conversation summaries or notifications, newest first.
    ListEntries {
        #[command(flatten)]
        vault: VaultArg,
        #[arg(long, value_enum)]
        kind: Kind,
        #[arg(long)]
        limit: Option<u32>,
    },

    /// Read one entry.
    ReadEntry {
        #[command(flatten)]
        vault: VaultArg,
        #[arg(long, value_enum)]
        kind: Kind,
        /// A filename from list-entries.
        #[arg(long)]
        id: String,
        #[arg(long)]
        start_line: Option<u32>,
        #[arg(long)]
        max_lines: Option<u32>,
    },

    /// Create a conversation summary or notification. Content comes from stdin.
    CreateEntry {
        #[command(flatten)]
        vault: VaultArg,
        #[arg(long, value_enum)]
        kind: Kind,
    },

    /// Replace or create one ordinary note. Content comes from stdin.
    WriteDocument {
        #[command(flatten)]
        vault: VaultArg,
        /// Vault-relative Markdown path. Never inside aios/.
        #[arg(long)]
        path: String,
        /// The revision from the latest read of this file.
        #[arg(long, value_name = "REVISION")]
        expected_revision: Option<String>,
        /// Create a note that does not exist yet.
        #[arg(long, conflicts_with = "expected_revision")]
        create: bool,
    },

    /// Create a folder for ordinary notes.
    CreateFolder {
        #[command(flatten)]
        vault: VaultArg,
        /// Vault-relative folder path. Never inside aios/.
        #[arg(long)]
        path: String,
    },

    /// Rename or move a note or folder. Cannot cross the aios/ boundary.
    MovePath {
        #[command(flatten)]
        vault: VaultArg,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
    },

    /// Move a note or folder into the vault's .trash/ folder.
    DeletePath {
        #[command(flatten)]
        vault: VaultArg,
        #[arg(long)]
        path: String,
        /// The revision from the latest read, for a file.
        #[arg(long, value_name = "REVISION")]
        expected_revision: Option<String>,
    },

    /// Build the vault's whole link graph in one call.
    LinkGraph {
        #[command(flatten)]
        vault: VaultArg,
        /// Leave the protected aios/ tree out of the graph.
        #[arg(long)]
        exclude_aios: bool,
        #[arg(long)]
        max_depth: Option<u32>,
    },

    /// Replace one entry, keeping Heimdall's frontmatter. Content comes from stdin.
    WriteEntry {
        #[command(flatten)]
        vault: VaultArg,
        #[arg(long, value_enum)]
        kind: Kind,
        /// A filename from list-entries.
        #[arg(long)]
        id: String,
        /// The revision from the latest read.
        #[arg(long, value_name = "REVISION")]
        expected_revision: String,
    },
}

/// The vault every domain command operates on.
///
/// Required, with no working-directory fallback, matching the rule the MCP
/// server follows (SPEC §7): the vault is always explicit.
#[derive(Debug, Args)]
struct VaultArg {
    /// Path to the vault.
    #[arg(long, value_name = "PATH")]
    vault: Utf8PathBuf,
}

impl VaultArg {
    fn open(&self) -> Result<Vault> {
        Vault::open(&self.vault)
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Kind {
    Conversation,
    Notification,
}

impl From<Kind> for EntryKind {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Conversation => Self::Conversation,
            Kind::Notification => Self::Notification,
        }
    }
}

pub fn run() -> ExitCode {
    let cli = Cli::parse();

    if cli.version {
        return version(cli.json);
    }

    let Some(command) = cli.command else {
        // Running bare prints help and mutates nothing (SPEC §11).
        let _ = Cli::command().print_help();
        println!();
        return ExitCode::SUCCESS;
    };

    match command {
        // The MCP server owns stdout for protocol traffic, so its failures are
        // reported on stderr rather than as a shell envelope: a client reading
        // the stream must never find one there.
        Command::Mcp { vault } => match mcp::serve(&vault) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("heimdall mcp: {}: {}", error.code, error.message);
                ExitCode::from(envelope::EXIT_DOMAIN_ERROR)
            }
        },
        command => match dispatch(command) {
            Ok(code) => code,
            Err(error) => envelope::failure(&error),
        },
    }
}

fn dispatch(command: Command) -> Result<ExitCode> {
    Ok(match command {
        Command::Mcp { .. } => {
            return Err(Error::internal(
                "the MCP server is handled before the shell envelope",
            ))
        }

        Command::Create { name, root } => {
            let root = match root {
                Some(root) => root,
                None => working_directory()?,
            };
            envelope::ok(&commands::create_vault(commands::CreateVaultRequest {
                name,
                root,
            })?)
        }

        Command::ListDocuments {
            vault,
            path,
            recursive,
            max_depth,
            cursor,
            limit,
        } => envelope::ok(&commands::list_documents(
            &vault.open()?,
            commands::ListDocumentsRequest {
                path,
                recursive,
                max_depth,
                cursor,
                limit,
            },
        )?),

        Command::ReadDocuments {
            vault,
            doc,
            start_line,
            max_lines,
            max_total_bytes,
            request,
        } => {
            let request = if request {
                serde_json::from_str(&read_stdin()?).map_err(|err| {
                    Error::invalid_input(format!("stdin is not a valid request: {err}"))
                })?
            } else {
                commands::ReadDocumentsRequest {
                    documents: doc
                        .into_iter()
                        .map(|path| commands::DocumentSelection {
                            path,
                            start_line,
                            max_lines,
                        })
                        .collect(),
                    max_total_bytes,
                }
            };
            envelope::ok(&commands::read_documents(&vault.open()?, request)?)
        }

        Command::ListMemories { vault, limit } => envelope::ok(&commands::list_memories(
            &vault.open()?,
            commands::ListMemoriesRequest { limit },
        )?),

        Command::ReadMemory {
            vault,
            extended,
            start_line,
            max_lines,
        } => envelope::ok(&commands::read_memory(
            &vault.open()?,
            commands::ReadMemoryRequest {
                extended,
                start_line,
                max_lines,
            },
        )?),

        Command::WriteMemory {
            vault,
            extended,
            expected_revision,
            create,
        } => {
            // The contract distinguishes "replace this exact revision" from
            // "create a file that does not exist yet"; --create is how the
            // shell says the latter, and neither flag is an error.
            let expected_revision = match (expected_revision, create) {
                (Some(raw), _) => Some(Some(raw.parse::<Revision>()?)),
                (None, true) => Some(None),
                (None, false) => None,
            };
            envelope::ok(&commands::write_memory(
                &vault.open()?,
                commands::WriteMemoryRequest {
                    content: read_stdin()?,
                    extended,
                    expected_revision,
                },
            )?)
        }

        Command::ListEntries { vault, kind, limit } => envelope::ok(&commands::list_entries(
            &vault.open()?,
            commands::ListEntriesRequest {
                kind: kind.into(),
                limit,
            },
        )?),

        Command::ReadEntry {
            vault,
            kind,
            id,
            start_line,
            max_lines,
        } => envelope::ok(&commands::read_entry(
            &vault.open()?,
            commands::ReadEntryRequest {
                kind: kind.into(),
                id,
                start_line,
                max_lines,
            },
        )?),

        Command::CreateEntry { vault, kind } => envelope::ok(&commands::create_entry(
            &vault.open()?,
            commands::CreateEntryRequest {
                kind: kind.into(),
                content: read_stdin()?,
            },
        )?),

        Command::WriteDocument {
            vault,
            path,
            expected_revision,
            create,
        } => {
            // The same three-state contract write-memory uses: a revision to
            // replace, --create for a file that does not exist yet, and neither
            // as the caller mistake it is.
            let expected_revision = match (expected_revision, create) {
                (Some(raw), _) => Some(Some(raw.parse::<Revision>()?)),
                (None, true) => Some(None),
                (None, false) => None,
            };
            envelope::ok(&commands::write_document(
                &vault.open()?,
                commands::WriteDocumentRequest {
                    path,
                    content: read_stdin()?,
                    expected_revision,
                },
            )?)
        }

        Command::CreateFolder { vault, path } => envelope::ok(&commands::create_folder(
            &vault.open()?,
            commands::CreateFolderRequest { path },
        )?),

        Command::MovePath { vault, from, to } => envelope::ok(&commands::move_path(
            &vault.open()?,
            commands::MovePathRequest { from, to },
        )?),

        Command::DeletePath {
            vault,
            path,
            expected_revision,
        } => envelope::ok(&commands::delete_path(
            &vault.open()?,
            commands::DeletePathRequest {
                path,
                expected_revision: expected_revision
                    .map(|raw| raw.parse::<Revision>())
                    .transpose()?,
            },
        )?),

        // The flag is spelled as the non-default because a bare clap flag can
        // only express one, and including the protected tree is the default the
        // desktop's file tree needs.
        Command::LinkGraph {
            vault,
            exclude_aios,
            max_depth,
        } => envelope::ok(&commands::link_graph(
            &vault.open()?,
            commands::LinkGraphRequest {
                include_aios: !exclude_aios,
                max_depth,
            },
        )?),

        Command::WriteEntry {
            vault,
            kind,
            id,
            expected_revision,
        } => envelope::ok(&commands::write_entry(
            &vault.open()?,
            commands::WriteEntryRequest {
                kind: kind.into(),
                id,
                content: read_stdin()?,
                expected_revision: expected_revision.parse::<Revision>()?,
            },
        )?),
    })
}

/// Read Markdown content from stdin, bounded before it reaches a domain check.
///
/// Content never arrives as a shell-interpreted argument (SPEC §11): a note
/// full of quotes, backticks, and newlines has to survive verbatim.
fn read_stdin() -> Result<String> {
    let mut buffer = Vec::new();
    std::io::stdin()
        .lock()
        .take(limits::STDIN_MAX_BYTES as u64 + 1)
        .read_to_end(&mut buffer)
        .map_err(|err| Error::io_error(format!("failed to read stdin: {}", err.kind())))?;

    if buffer.len() > limits::STDIN_MAX_BYTES {
        return Err(Error::limit_exceeded(format!(
            "stdin is larger than the {} byte maximum",
            limits::STDIN_MAX_BYTES
        ))
        .with_detail("maximum", limits::STDIN_MAX_BYTES));
    }
    String::from_utf8(buffer)
        .map_err(|_| Error::invalid_input("stdin is not valid UTF-8; Markdown must be UTF-8"))
}

fn working_directory() -> Result<Utf8PathBuf> {
    let cwd = std::env::current_dir()
        .map_err(|err| Error::io_error(format!("cannot read the working directory: {}", err.kind())))?;
    Utf8PathBuf::from_path_buf(cwd)
        .map_err(|_| Error::invalid_input("the working directory is not valid UTF-8; pass --root"))
}

fn version(as_json: bool) -> ExitCode {
    let cli_version = env!("CARGO_PKG_VERSION");
    if !as_json {
        println!("heimdall {cli_version}");
        return ExitCode::SUCCESS;
    }
    envelope::ok(&json!({
        "cli_version": cli_version,
        "core_version": heimdall_core::VERSION,
        "output_schema_version": envelope::SCHEMA_VERSION,
        "mcp_protocol_version": crate::mcp::PROTOCOL_VERSION.to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_surface_is_valid_and_named_heimdall() {
        Cli::command().debug_assert();
        assert_eq!(Cli::command().get_name(), "heimdall");
    }

    #[test]
    fn every_domain_subcommand_requires_a_vault() {
        let command = Cli::command();
        for subcommand in command.get_subcommands() {
            // `create` names a location with --root instead of selecting a vault.
            if subcommand.get_name() == "create" {
                continue;
            }
            let vault = subcommand
                .get_arguments()
                .find(|arg| arg.get_id() == "vault")
                .unwrap_or_else(|| panic!("{} has no --vault", subcommand.get_name()));
            assert!(
                vault.is_required_set(),
                "{} has an optional --vault",
                subcommand.get_name()
            );
        }
    }

    #[test]
    fn subcommand_names_are_kebab_case() {
        for subcommand in Cli::command().get_subcommands() {
            let name = subcommand.get_name();
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{name} is not kebab-case"
            );
        }
    }

    #[test]
    fn every_shell_operation_is_reachable() {
        let command = Cli::command();
        let names: Vec<_> = command.get_subcommands().map(|s| s.get_name()).collect();
        for expected in [
            // The MCP surface, mirrored one-to-one (SPEC §9).
            "list-documents",
            "read-documents",
            "list-memories",
            "read-memory",
            "write-memory",
            "list-entries",
            "read-entry",
            "create-entry",
            // Client operations: shell-only, never tools (SPEC §7, §15).
            "create",
            "write-document",
            "create-folder",
            "move-path",
            "delete-path",
            "write-entry",
            "link-graph",
            "mcp",
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }
        // No general-purpose file access ever appears here (SPEC §9).
        for forbidden in ["read-file", "write-file", "execute"] {
            assert!(!names.contains(&forbidden), "unexpected {forbidden}");
        }
    }
}
