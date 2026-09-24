# Heimdall

An intent-aware layer over Markdown vaults. Heimdall gives AI clients two verbs
over your notes — `read` and `write` — instead of unrestricted filesystem
access, and lets you **lock** any note, any folder, or the whole vault so that
nothing can change it until you unlock it.

A vault is a folder of Markdown files and nothing else, so any Markdown editor
opens the same files. Heimdall keeps nothing of its own inside it.

`docs/SPEC.md` is the source of truth for product behavior.

## Status

**Phases 1–3, 5, and 6 (SPEC §18) are implemented**: `heimdall-core`, the direct
shell CLI, the MCP stdio server, the Tauri desktop app, the desktop editor with
its link graph, and read/write/locks. Phase 4 — signed artifacts and installers —
is still to come.

## Build

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Use

Create a vault, or register a folder of notes you already have (nothing is
written into it):

```bash
heimdall create my-vault --root ~/Documents
```

Inside a vault, `read`, `write`, `lock`, and `unlock` need no configuration.
They work on the current folder by default, and paths are relative to it:

```bash
cd ~/Documents/my-vault/projects

heimdall read                              # list this folder
heimdall read --recursive                  # ...and everything beneath it
heimdall read my_project.md                # read a note (bounded; follow next_line)
heimdall read ../ideas/hello_world.md      # anywhere else in the vault

echo "# Plan" | heimdall write plan.md     # create a note; never replaces one
echo "# Plan v2" | heimdall write plan.md --expected-revision "blake3:..."

heimdall lock                              # lock this folder
heimdall unlock my_project.md              # ...but let this one note be edited
heimdall lock ..                           # lock the whole vault
```

From anywhere else, name the vault with `--vault`; paths are then relative to
the vault root. The full path to a note works too. Every command prints a JSON
envelope on stdout, and Markdown content always arrives on stdin, never as a
shell-interpreted argument.

A locked note can be read but not written, moved, or deleted — by an agent, a
script, or the desktop editor — and every read says whether a path is locked.
Locking a folder locks everything in it; a single note can then be unlocked
inside it.

Exit codes: `0` success, `1` domain error (the error envelope is still printed),
`2` usage error.

## Use from an AI client

One server process per vault, over stdio:

```bash
heimdall mcp --vault ~/Documents/my-vault
```

```json
{
  "mcpServers": {
    "heimdall": {
      "command": "heimdall",
      "args": ["mcp", "--vault", "/Users/name/Documents/my-vault"]
    }
  }
}
```

Four tools: `read`, `write`, `lock`, `unlock` — the same four verbs as the
shell. There is no general file read, write, or execute tool, and no tool takes
a vault path — the vault is fixed by `--vault` and cannot be named, switched, or
discovered by a call. Creating a vault stays a shell command.

The server publishes an `instructions` string at initialization telling a client
to start with `read`, to pass the revision from its latest read when it writes,
and that a locked path is the user's decision: it must not `unlock` anything it
was not asked to. A lock is therefore a guardrail against mistakes rather than a
wall against a hostile agent, since `unlock` is a tool too.

Each tool publishes an input and output schema derived from the same Rust types
the shell returns, so the two adapters cannot drift. Results come back as typed
`structuredContent` rather than the shell envelope. Expected refusals are tool
results flagged `isError` carrying a domain `code` (`LOCKED`,
`REVISION_CONFLICT`, …), `message`, and safe `details`; protocol errors are
reserved for calls the server cannot execute at all. Pinned to `rmcp` 3.1.3 and
MCP protocol `2025-11-25`; check with:

```bash
heimdall --version --json
```

## Desktop app

A three-pane workspace over the same vault.

```bash
cd apps/desktop
npm install
npm run sidecar        # build the CLI and stage it as a Tauri externalBin
npm run tauri:dev      # or: npm run tauri:build
```

Three panes. **Left** is the whole vault, with new note, new folder, sort,
collapse-all, and a lock for the whole vault; right-click for rename, delete,
and lock or unlock. Locked notes and folders carry a small lock glyph.
**Middle** is the note: a breadcrumb with back/forward, a lock toggle, and a
switch between a Markdown source editor and a rendered preview that shows YAML
frontmatter as a properties table, draws `mermaid` diagrams, and lists linked
mentions. A locked note opens read-only. **Right** is a force-directed graph of
the vault: pan with inertia, zoom about the pointer, drag a node and its
neighbours follow, click one to open it, hover to light up its neighbourhood and
dim the rest. `⌘O` finds a note by name.

The note's heading is its filename — editing one renames the other.

Dark mode is pure black behind white; light mode is the exact inverse. The
system preference decides unless overridden. Each theme has its own accent
colour, set in Settings, spent on links, the open note, the active graph node,
and mermaid. Until you pick one that theme is monochrome — black links on white,
white on black.

**Heimdall → Settings…** (`⌘,`) opens a modal with four sections: **Vault**
creates a templated vault or opens a folder you already have; **Server** shows
the exact `heimdall mcp` command, generates the `mcpServers` snippet, offers to
write it into a detected client's configuration, and proves it works with a real
MCP handshake; **Appearance** overrides the theme; **Diagnostics** reports which
binary is in use, its versions, the vault's state, the enforced limits, and
recent failures.

The editor saves through `write`, exactly as an agent does, so a lock or a stale
revision refuses it too. The structural edits the MCP surface deliberately does
not have are shell subcommands the desktop calls instead — `create-folder`,
`move-path`, `relink`, `delete-path`, and `link-graph`. None of them is an MCP
tool, and none of their types derives `JsonSchema`, so giving one a tool would
not compile. The tool surface stays at four. Deleting moves a note into the
vault's `.trash/`; nothing is ever unlinked.

The app always runs the version-matched CLI bundled inside it — never one found
on `PATH` — and never uses a shell. React cannot name an executable or invent a
flag: it calls a Tauri command whose subcommand and argument keys are both
allowlisted in Rust.

## Design in one paragraph

`heimdall-core` owns every filesystem operation; adapters only translate into it.
The vault is opened once as a `cap-std` directory capability, so `..`, absolute
children, and symlinks pointing out of the vault fail at the syscall boundary
rather than being filtered by hand. No operation reads a whole vault: listings
paginate and reads are bounded by line and byte budgets, and partial results
always say so. Replacing a note requires the revision from the latest read,
compared, checked against the lock rules, and written inside one cross-process
lock; a write without a revision only ever creates. The write lock, the lock
rules, and the registry of known vaults all live in the per-user
application-data directory, never in the vault. The one operation that does read
the whole vault — the link graph — is capped by node, per-file, and total-byte
budgets, returns metadata and link endpoints but never content, and reports
exactly what it left out.
