# Heimdall

An intent-aware layer over Obsidian-compatible Markdown vaults. Heimdall gives AI
clients constrained, purpose-aware operations over notes, agent instructions,
durable memories, and entries — instead of unrestricted filesystem access.

`docs/SPEC.md` is the source of truth for product behavior.

## Status

**Phases 1–3 and 5 (SPEC §18) are implemented**: `heimdall-core`, the direct shell
CLI, the MCP stdio server, the Tauri desktop app, and the desktop editor — its
client write operations, the link graph, and the three-pane workspace. Phase 4 —
signed artifacts and installers — is still to come.

## Build

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Use

Create a vault, or add the managed structure to an existing Obsidian vault:

```bash
heimdall create my-vault --root ~/Documents
```

Every domain command takes an explicit `--vault` and prints a JSON envelope on
stdout:

```bash
V=~/Documents/my-vault

heimdall list-documents --vault "$V" --recursive
heimdall read-documents --vault "$V" --doc ideas/hello_world.md
heimdall read-agents    --vault "$V"

heimdall list-memories  --vault "$V"
heimdall read-memory    --vault "$V"
echo "# Memory" | heimdall write-memory --vault "$V" --expected-revision "blake3:..."
echo "# Topic"  | heimdall write-memory --vault "$V" --extended topic.md --create

heimdall list-entries --vault "$V" --kind conversation
echo "# Summary" | heimdall create-entry --vault "$V" --kind conversation
heimdall read-entry --vault "$V" --kind conversation --id 2026-08-16_10-30-00.md
```

Markdown content always arrives on stdin, never as a shell-interpreted argument.

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

Nine tools: `list_documents`, `read_documents`, `read_agents`, `list_memories`,
`read_memory`, `write_memory`, `list_entries`, `read_entry`, `create_entry`.
There is no general file read, write, or execute tool, and no tool takes a vault
path — the vault is fixed by `--vault` and cannot be named, switched, or
discovered by a call. Creating a vault stays a shell command.

Each tool publishes an input and output schema derived from the same Rust types
the shell returns, so the two adapters cannot drift. Results come back as typed
`structuredContent` rather than the shell envelope. Expected refusals are tool
results flagged `isError` carrying a domain `code`, `message`, and safe
`details`; protocol errors are reserved for calls the server cannot execute at
all. Pinned to `rmcp` 3.1.3 and MCP protocol `2025-11-25`; check with:

```bash
heimdall --version --json
```

## Desktop app

An Obsidian-style workspace over the same vault.

```bash
cd apps/desktop
npm install
npm run sidecar        # build the CLI and stage it as a Tauri externalBin
npm run tauri:dev      # or: npm run tauri:build
```

Three panes. **Left** is the whole vault including `aios/`, with new note, new
folder, sort, and collapse-all, and rename/delete on right-click. **Middle** is the note: a breadcrumb with
back/forward, and a toggle between a Markdown source editor and a rendered
preview that shows YAML frontmatter as a properties table, draws `mermaid`
diagrams, and lists linked mentions. **Right** is a force-directed graph of the
vault, built to match Obsidian's: pan with inertia, zoom about the pointer, drag
a node and its neighbours follow, click one to open it, hover to light up its
neighbourhood and dim the rest. `⌘O` finds a note by name.

The note's heading is its filename — editing one renames the other.

Dark mode is pure black with `#00ff00` links; light mode is the exact inverse
with black links. The system preference decides unless overridden.

**Heimdall → Settings…** (`⌘,`) opens a modal with four sections: **Vault**
creates a templated vault or initializes an existing Obsidian one; **Server**
shows the exact `heimdall mcp` command, generates the `mcpServers` snippet, offers
to write it into a detected client's configuration, and proves it works with a
real MCP handshake; **Appearance** overrides the theme; **Diagnostics** reports
which binary is in use, its versions, the vault's state, the enforced limits, and
recent failures.

Editing needs writes the MCP surface deliberately does not have, so the desktop
calls shell subcommands instead — `write-document`, `create-folder`, `move-path`,
`delete-path`, `write-agents`, `write-entry`, and `link-graph`. None of them is
an MCP tool, and none of their types derives `JsonSchema`, so giving one a tool
would not compile. The tool surface stays at nine. Saving carries the revision
the note was read at, and a stale one becomes a conflict the user resolves —
never a silent overwrite. Deleting moves a note into the vault's `.trash/`;
nothing is ever unlinked.

The app always runs the version-matched CLI bundled inside it — never one found
on `PATH` — and never uses a shell. React cannot name an executable or invent a
flag: it calls a Tauri command whose subcommand and argument keys are both
allowlisted in Rust.

## Design in one paragraph

`heimdall-core` owns every filesystem operation; adapters only translate into it.
The vault is opened once as a `cap-std` directory capability, so `..`, absolute
children, and symlinks pointing out of the vault fail at the syscall boundary
rather than being filtered by hand. Ordinary document operations never see the
protected `aios/` tree. No operation reads a whole vault: listings paginate and
reads are bounded by line and byte budgets, and partial results always say so.
Replacing a memory requires the revision from the latest read, compared and
written inside one cross-process lock; entries are create-only over MCP and never
overwrite. The desktop's own write path obeys the same revision rule, and the one
operation that does read the whole vault — the link graph — is capped by node,
per-file, and total-byte budgets, returns metadata and link endpoints but never
content, and reports exactly what it left out.
