//! The direct shell command surface (SPEC §9, §11).
//!
//! Subcommands are kebab-case and mirror the core operations one-to-one, so the
//! MCP adapter can be checked against this surface for equivalent domain
//! outcomes. `read` and `write` are the two the MCP server also offers; `lock`
//! and `unlock` are a person's, and the rest are the desktop's client
//! operations and the choice of which vaults AI clients may see.

use std::io::Read;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use clap::{Args, CommandFactory, Parser, Subcommand};
use serde_json::json;
use heimdall_core::commands;
use heimdall_core::{agents, appdata, limits, registry, Error, RelPath, Result, Revision, Vault};

use crate::presence::{self, Person};
use crate::{envelope, mcp};

#[derive(Debug, Parser)]
#[command(
    name = "heimdall",
    about = "An intent-aware layer over Markdown vaults.",
    long_about = "Heimdall gives AI clients two verbs over a Markdown vault — read and \
                  write — instead of unrestricted filesystem access, and lets people lock \
                  notes and folders so nothing can write them.\n\nRun from inside a vault, \
                  read, write, lock, and unlock need no --vault and take paths relative to \
                  the current folder. Every command prints a JSON envelope on stdout.",
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
    /// Run the MCP server over stdio.
    ///
    /// Without --vault it serves every vault shared with AI clients (see
    /// `share`), and each tool call names the vault it means. With --vault it
    /// serves that one vault only.
    Mcp {
        /// Serve only this vault.
        #[arg(long, value_name = "PATH")]
        vault: Option<Utf8PathBuf>,
    },

    /// List the vaults Heimdall knows, and which are shared with AI clients.
    Vaults,

    /// Share a vault with AI clients, or rename it if it is already shared.
    Share {
        #[command(flatten)]
        vault: VaultArg,
        /// The name tool calls use for it. Defaults to the folder name.
        #[arg(long)]
        name: Option<String>,
    },

    /// Stop sharing a vault with AI clients.
    Unshare {
        #[command(flatten)]
        vault: VaultArg,
    },

    /// Create a new templated vault, or register an existing folder of notes as one.
    Create {
        /// One folder name, not a path.
        name: String,
        /// Where to create it. Defaults to the working directory.
        #[arg(long)]
        root: Option<Utf8PathBuf>,
    },

    /// Read a folder (a listing) or a note (a bounded range of its lines).
    ///
    /// With no PATH, reads the current folder. Inside a vault, paths are
    /// relative to the working directory, and --vault is not needed.
    Read {
        /// A folder or a Markdown note. Defaults to the current folder.
        path: Option<String>,
        #[command(flatten)]
        vault: FoundVault,
        /// Note: the 1-indexed first line to return.
        #[arg(long)]
        start_line: Option<u32>,
        /// Note: lines to return (default 200, maximum 1000).
        #[arg(long)]
        max_lines: Option<u32>,
        /// Note: content budget in bytes (default 65536, maximum 262144).
        #[arg(long)]
        max_total_bytes: Option<u32>,
        /// Folder: list everything beneath it, not just its children.
        #[arg(long)]
        recursive: bool,
        /// Folder: depth of a recursive listing (default 4, maximum 16).
        #[arg(long)]
        max_depth: Option<u32>,
        /// Folder: the next_cursor from a previous page.
        #[arg(long)]
        cursor: Option<String>,
        /// Folder: page size (default 50, maximum 200).
        #[arg(long)]
        limit: Option<u32>,
    },

    /// Create or replace one note. Content comes from stdin.
    ///
    /// Without --expected-revision it creates the note and refuses to replace
    /// one that exists; pass the revision from the latest read to replace it.
    Write {
        /// The Markdown note to write.
        path: String,
        #[command(flatten)]
        vault: FoundVault,
        /// The revision from the latest read of this note.
        #[arg(long, value_name = "REVISION")]
        expected_revision: Option<String>,
        /// Create a note that does not exist yet (the default without a revision).
        #[arg(long, conflicts_with = "expected_revision")]
        create: bool,
    },

    /// Lock a folder or note, making it read-only. With no PATH, the current folder.
    Lock {
        /// A folder or a Markdown note. Defaults to the current folder.
        path: Option<String>,
        #[command(flatten)]
        vault: FoundVault,
    },

    /// Unlock a folder or note, making it writable. With no PATH, the current folder.
    ///
    /// Asks for Touch ID or your password first: an unlock has to come from a
    /// person, not from a program acting for one.
    Unlock {
        /// A folder or a Markdown note. Defaults to the current folder.
        path: Option<String>,
        #[command(flatten)]
        vault: FoundVault,
    },

    /// Create a folder.
    CreateFolder {
        #[command(flatten)]
        vault: VaultArg,
        /// Vault-relative folder path.
        #[arg(long)]
        path: String,
    },

    /// Rename or move a note or folder.
    MovePath {
        #[command(flatten)]
        vault: VaultArg,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
    },

    /// Retarget the links that pointed at a path a move has just changed.
    ///
    /// Called after move-path, never instead of it. Only files holding a link
    /// to the moved path are written, and locked notes are never written.
    Relink {
        #[command(flatten)]
        vault: VaultArg,
        /// Where the note or folder was, before the move.
        #[arg(long)]
        from: String,
        /// Where it is now.
        #[arg(long)]
        to: String,
        /// Report what would change without writing anything.
        #[arg(long)]
        dry_run: bool,
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
        #[arg(long)]
        max_depth: Option<u32>,
    },
}

/// The vault a client operation works on. Required: these are the desktop's
/// commands, and the desktop always says which vault it means.
#[derive(Debug, Args)]
struct VaultArg {
    /// Path to the vault.
    #[arg(long, value_name = "PATH")]
    vault: Utf8PathBuf,
}

impl VaultArg {
    fn open(&self) -> Result<Vault> {
        open_explicit(&self.vault)
    }
}

/// The vault `read`, `write`, `lock`, and `unlock` work on: the one named by
/// `--vault`, or else the registered vault containing the working directory.
#[derive(Debug, Args)]
struct FoundVault {
    /// Path to the vault. Defaults to the vault containing the working directory.
    #[arg(long, value_name = "PATH")]
    vault: Option<Utf8PathBuf>,
}

impl FoundVault {
    /// Open the vault and turn the typed path into a vault path.
    ///
    /// Paths are relative to where the command stands: the working directory
    /// when the vault was found from it, the vault root when `--vault` named
    /// it. Omitting the path means that same place — the current folder.
    fn open(&self, path: Option<&str>) -> Result<(Vault, Option<String>)> {
        let (vault, base) = match &self.vault {
            Some(root) => (open_explicit(root)?, RelPath::root()),
            None => {
                let located = registry::locate(&working_directory()?, &appdata::data_dir()?)?;
                (located.vault, located.cwd)
            }
        };
        let resolved = match path {
            Some(raw) => registry::resolve_shell_path(vault.root(), &base, raw)?,
            None => base,
        };
        let path = (!resolved.is_root()).then(|| resolved.to_string());
        Ok((vault, path))
    }
}

/// Open a vault named with `--vault`, registering it so later commands can
/// find it from inside. Registration is a convenience, so it never fails the
/// command it rides along with.
fn open_explicit(root: &Utf8Path) -> Result<Vault> {
    let vault = Vault::open(root)?;
    let _ = registry::register(&vault);
    Ok(vault)
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
        Command::Mcp { vault } => match mcp::serve(vault.as_deref()) {
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

        Command::Read {
            path,
            vault,
            start_line,
            max_lines,
            max_total_bytes,
            recursive,
            max_depth,
            cursor,
            limit,
        } => {
            let (vault, path) = vault.open(path.as_deref())?;
            envelope::ok(&commands::read(
                &vault,
                commands::ReadRequest {
                    vault: None,
                    path,
                    start_line,
                    max_lines,
                    max_total_bytes,
                    recursive,
                    max_depth,
                    cursor,
                    limit,
                },
            )?)
        }

        Command::Write {
            path,
            vault,
            expected_revision,
            create: _,
        } => {
            // --create is the default spelled out; only a revision changes
            // what the write may do.
            let expected_revision = expected_revision
                .map(|raw| raw.parse::<Revision>())
                .transpose()?
                .map(Some);
            let (vault, path) = vault.open(Some(&path))?;
            let path = path.ok_or_else(|| {
                Error::invalid_input("write needs a note path, not a folder")
                    .with_detail("parameter", "path")
            })?;
            envelope::ok(&commands::write(
                &vault,
                commands::WriteRequest {
                    vault: None,
                    path,
                    content: read_stdin()?,
                    expected_revision,
                },
            )?)
        }

        Command::Lock { path, vault } => {
            let (vault, path) = vault.open(path.as_deref())?;
            envelope::ok(&commands::lock(&vault, commands::LockRequest { path })?)
        }

        Command::Unlock { path, vault } => {
            let (vault, path) = vault.open(path.as_deref())?;
            let person = Person::for_vault(&vault);
            envelope::ok(&commands::unlock(&vault, commands::LockRequest { path }, &person)?)
        }

        Command::Vaults => envelope::ok(&list_vaults()?),

        Command::Share { vault, name } => {
            let vault = vault.open()?;
            let shared = agents::share(&vault, name.as_deref())?;
            envelope::ok(&json!({ "name": shared.name, "path": shared.path, "shared": true }))
        }

        Command::Unshare { vault } => {
            let vault = vault.open()?;
            let changed = agents::unshare(&vault)?;
            envelope::ok(&json!({ "path": vault.root(), "shared": false, "changed": changed }))
        }

        Command::CreateFolder { vault, path } => envelope::ok(&commands::create_folder(
            &vault.open()?,
            commands::CreateFolderRequest { path },
        )?),

        Command::MovePath { vault, from, to } => envelope::ok(&commands::move_path(
            &vault.open()?,
            commands::MovePathRequest { from, to },
        )?),

        Command::Relink {
            vault,
            from,
            to,
            dry_run,
        } => envelope::ok(&commands::relink(
            &vault.open()?,
            commands::RelinkRequest { from, to, dry_run },
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

        Command::LinkGraph { vault, max_depth } => envelope::ok(&commands::link_graph(
            &vault.open()?,
            commands::LinkGraphRequest { max_depth },
        )?),
    })
}

/// Every vault Heimdall knows — registered, shared, or both — with whether it
/// is shared with AI clients and under what name.
///
/// Shell output, for a person and the desktop; it names absolute paths, which
/// is why nothing like it is ever an MCP tool.
fn list_vaults() -> Result<serde_json::Value> {
    let data_dir = appdata::data_dir()?;
    let shared = agents::list(&data_dir)?;
    let mut roots = registry::list(&data_dir)?;
    for vault in &shared {
        if !roots.contains(&vault.path) {
            roots.push(vault.path.clone());
        }
    }
    roots.sort();

    let vaults: Vec<_> = roots
        .into_iter()
        .map(|root| {
            let share = shared.iter().find(|vault| vault.path == root);
            json!({
                "path": root,
                "folder": root.file_name().unwrap_or_default(),
                "exists": root.is_dir(),
                "shared": share.is_some(),
                "name": share.map(|vault| vault.name.clone()),
            })
        })
        .collect();
    Ok(json!({ "vaults": vaults }))
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
        // A release build always asks a person before an unlock; the release
        // script refuses to ship one that does not say so.
        "presence": if presence::REQUIRED { "required" } else { "scripted-in-debug" },
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
    fn client_operations_require_a_vault_and_the_four_verbs_find_one() {
        let command = Cli::command();
        for subcommand in command.get_subcommands() {
            let name = subcommand.get_name();
            // `create` names a location with --root instead of selecting a
            // vault, and `vaults` lists them all.
            if name == "create" || name == "vaults" {
                continue;
            }
            let vault = subcommand
                .get_arguments()
                .find(|arg| arg.get_id() == "vault")
                .unwrap_or_else(|| panic!("{name} has no --vault"));
            // `mcp` without --vault serves the shared vaults.
            let discovers = matches!(name, "read" | "write" | "lock" | "unlock" | "mcp");
            assert_eq!(
                vault.is_required_set(),
                !discovers,
                "{name}: --vault should be {}",
                if discovers { "optional" } else { "required" }
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
            "read",
            "write",
            "lock",
            "unlock",
            // Client operations: shell-only, never tools (SPEC §7, §15).
            "create",
            "create-folder",
            "move-path",
            "relink",
            "delete-path",
            "link-graph",
            "mcp",
            // Which vaults AI clients may see: a person's choice, never a tool.
            "vaults",
            "share",
            "unshare",
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }
        assert_eq!(names.len(), 14, "unexpected subcommands: {names:?}");
        // No general-purpose file access ever appears here (SPEC §9).
        for forbidden in ["read-file", "write-file", "execute", "read_file", "write_file"] {
            assert!(!names.contains(&forbidden), "unexpected {forbidden}");
        }
    }
}
