# Heimdall Application Specification

> Product name and installed executable: `heimdall`.
> This document is the source of truth for product behavior. `AGENTS.md` summarizes working conventions and links here.

## 1. Product definition

Heimdall is an intent-aware MCP layer for Markdown vaults. It lets AI clients work with notes, durable memories, and entries (conversation summaries and notifications) through constrained domain operations instead of unrestricted filesystem access.

Heimdall has two independently installable products:

1. **Heimdall CLI** — a headless Rust executable providing direct shell commands and an MCP server. The CLI is the product core; it is fully usable without the desktop application.
2. **Heimdall Desktop** — a Tauri application with a React, TypeScript, and Vite interface. It both configures the server and works the vault: a file tree, a Markdown editor with preview, and an interactive link graph, alongside vault creation, MCP client configuration, and diagnostics. A vault is a folder of plain Markdown files and nothing else, so any Markdown editor remains a perfectly good way to work over the same files.

The desktop application bundles a version-matched CLI and calls it for every domain operation.

## 2. Terminology

- **Vault** — a folder of Markdown files, or a user-selected folder inside one. It is the security and content boundary for a Heimdall session.
- **Ordinary document** — a Markdown note outside the protected `aios/` folder.
- **Managed content** — memories, entries, and attachments inside `aios/`.
- **Entry** — a create-only, timestamped managed record. V1 has two entry kinds: `conversation` (a compressed conversation summary) and `notification`.
- **Revision** — a BLAKE3 hash of the exact file bytes, formatted as `blake3:<lowercase-hex>`.
- **Client operation** — a domain operation the desktop calls directly through the shell CLI and the MCP surface does not expose. `heimdall create` (§7) is the original; the editor's writes and `link_graph` are the rest (§15). The distinction is enforced by the type system, not by convention: a client operation's request and response types derive no `JsonSchema`, and `rmcp` cannot build a tool without one.

"Application" refers only to the Heimdall desktop product. User content locations are called vaults.

## 3. Design principles

- Keep one implementation of domain and filesystem behavior in `heimdall-core`.
- Give AI clients purpose-aware operations, not arbitrary file access.
- List before reading and read only selected, bounded content.
- Keep ordinary notes separate from protected Heimdall state.
- One vault per server process; scope is fixed by configuration, never by tool input.
- Never overwrite user data silently.
- Use optimistic concurrency for replace-style writes.
- Prefer stateless continuation over server-side cursor state.
- Use structured, versioned output at process boundaries.
- Default to local-only operation; no network or telemetry is required.
- Add databases, daemons, or general write operations only when measured requirements justify them.

## 4. System architecture

```text
MCP client                         Desktop user
    |                                   |
    | MCP over stdio                    | React UI
    v                                   v
+------------------+              +----------------------+
| heimdall CLI      |<-------------| Tauri bridge         |
| - shell commands | JSON/stdin   | bundled CLI sidecar  |
| - MCP adapter    |              | editor + diagnostics |
+---------+--------+              +----------------------+
          |
          v
+------------------+
| heimdall-core     |
| domain rules     |
| bounded reads    |
| secure writes    |
+---------+--------+
          |
          v
 Markdown vaults
```

### Boundaries

- `heimdall-core` owns validation, limits, revisions, and filesystem operations.
- `heimdall-cli` translates shell commands and MCP calls into core operations.
- The Tauri Rust layer only launches the bundled CLI, supplies input, and returns output.
- Client operations exist only on the shell surface. Adding one never adds an MCP tool.
- React never accesses the filesystem or launches arbitrary processes.
- The desktop calls shell commands directly; it does not use MCP internally.

## 5. Repository layout

Use one Cargo workspace with the desktop frontend in the same repository.

```text
heimdall/
├── Cargo.toml
├── Cargo.lock
├── AGENTS.md
├── crates/
│   ├── heimdall-core/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── commands/
│   │       ├── errors.rs
│   │       ├── limits.rs
│   │       ├── paths.rs
│   │       ├── revisions.rs
│   │       ├── template.rs
│   │       ├── timestamps.rs
│   │       └── storage.rs
│   └── heimdall-cli/
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs
│           ├── shell.rs
│           └── mcp.rs
├── apps/
│   └── desktop/
│       ├── package.json
│       ├── vite.config.ts
│       ├── index.html
│       ├── src/
│       │   ├── main.tsx
│       │   ├── App.tsx
│       │   ├── api/
│       │   ├── components/
│       │   └── features/
│       ├── scripts/            # stages the version-matched sidecar
│       └── src-tauri/
│           ├── Cargo.toml
│           ├── tauri.conf.json
│           ├── capabilities/
│           ├── binaries/       # heimdall-<target-triple>, staged at build time
│           ├── tests/
│           └── src/
│               ├── lib.rs
│               ├── cli_bridge.rs
│               └── client_config.rs
├── docs/
│   ├── SPEC.md
│   └── vault-template/        # canonical default vault contents (see §7)
└── README.md
```

Integration tests live in each crate's own `tests/` directory, because Cargo
only compiles `tests/` inside a package and the workspace root is a virtual
manifest. `crates/heimdall-core/tests/` covers discovery bounds, durability, and
concurrency; `crates/heimdall-cli/tests/` covers the output envelope,
cross-process behavior, and the shell/MCP contract, with shared helpers in
`tests/common/`. Fixtures live beside the tests that use them.

The contract tests drive a real `heimdall mcp` process over stdio with a real
MCP client, so anything that corrupted the protocol stream shows up as a failed
handshake rather than a passing assertion.

`storage.rs` owns the vault capability itself (`Vault`) alongside atomic writes
and file locks; there is no separate vault module. `timestamps.rs` holds UTC
formatting and the entry-filename parser. `commands/links.rs` holds the link
scanner and `commands/frontmatter.rs` the entry-frontmatter parser, both
`pub(crate)` helpers in the same shape as `read_range.rs` — pure string in,
structure out, so the rules about fenced code and owned fields can be pinned down
without building a vault. Neither needs a new dependency.

`apps/desktop/src-tauri` is excluded from the Cargo workspace. `tauri::
generate_context!` embeds the built frontend, so a workspace member would break
`cargo build --workspace` until Vite has run, and the exclusion also keeps the
headless CLI artifact free of any WebView dependency (§16). The desktop crate
depends on `tauri` and `serde` only — never on `heimdall-core` — so its route to
the domain is the CLI's published contract and nothing else.

Desktop tests split the same way: `src-tauri/tests/` drives the real staged
sidecar (including a decoy `heimdall` placed earlier on `PATH`, to prove it is
never chosen), while `src/**/*.test.ts` covers the screens and the visual rules.

Recommended baseline dependencies:

- Rust: `clap`, `serde`, `serde_json`, `thiserror`, `camino`, `cap-std`, `blake3`, `dirs`, `fs4`, `include_dir`, `time`, and `tempfile`. (`fs4` is the maintained fork of the unmaintained `fs2`.) `dirs` locates the per-user application-data directory the write lock lives in (§14). `time` supplies the UTC formatting that entry filenames and frontmatter require.
- `tokio` belongs to `heimdall-cli` only, where `rmcp` needs it. `heimdall-core` is synchronous: its work is filesystem-bound, and a blocking API keeps it testable without a runtime. The MCP adapter runs each core call on a blocking worker rather than colouring the domain layer async.
- `schemars` is a core dependency so tool schemas derive from the domain types themselves (§12).
- MCP: the official Rust SDK, `rmcp`, pinned to a tested release and protocol baseline (`=3.1.3`, protocol `2025-11-25`).
- Desktop: Tauri 2, React, TypeScript, and Vite.
- Add frontend state management only when React context and local state become insufficient.

## 6. Vault contract

```text
<vault>/
├── *.md
├── <arbitrary files>
├── <ordinary-folders>/
│   ├── *.md
    └── <arbitrary files>
├── .trash/                     # deleted content; never listed, never read
└── aios/
    ├── memories/
    │   ├── memory.md
    │   └── extended/
    │       └── *.md
    ├── conversations/
    │   └── YYYY-MM-DD_HH-mm-ss.md
    ├── notifications/
    │   └── YYYY-MM-DD_HH-mm-ss.md
    └── attachments/
        └── <arbitrary files>
```

Rules:

- All managed text files use UTF-8 Markdown and the `.md` extension.
- Vault may contain non-Markdown files.
- Vault creation and initialization preserve every existing file, including hidden configuration folders belonging to other tools.
- Ordinary document operations exclude the entire `aios/` tree. The comparison is case-insensitive: on macOS and Windows `AIOS/notes.md` and `aios/notes.md` are the same file, so a case-sensitive check would let ordinary operations reach protected content.
- Only protected-content operations may access `aios/`.
- V1 does not expose attachment content.
- Generated Markdown must render normally in any ordinary Markdown reader.
- Memory links use wikilinks, for example:

```markdown
- [[aios/memories/extended/project_history|Project history]]
```

### Entry files and frontmatter

Generated entries receive Heimdall-owned frontmatter:

```markdown
---
created_at: 2026-08-16T14:30:00Z
type: conversation
---

# Summary

Compressed summary content.
```

- `type` is `conversation` or `notification` and always matches the folder the entry lives in.
- Heimdall owns entry frontmatter exclusively. If caller-supplied entry content itself begins with a frontmatter block (a leading `---` line), reject the call with `INVALID_INPUT` and a message explaining that Heimdall adds frontmatter. Never strip, merge, or silently rewrite supplied content.
- A replace-style entry write (`write_entry`, §15) reads the invariant more precisely than the create rule can. What Heimdall owns is `created_at` and `type`, which must always agree with the filename and the folder; the rest of the block is the user's. So an edit must carry byte-identical values for both owned fields, and any other key — `tags`, a `links` list from an editor's property panel — passes through untouched. An entry with no Heimdall frontmatter, such as one copied in by hand, has nothing to preserve and may be edited freely.
- Deleted content moves to `.trash/` at the vault root, mirroring its original folders. The leading dot means it is already excluded from every listing, and mirroring is the only layout in which restoring a file is unambiguous. Emptying the trash is not a Heimdall operation.
- Filename timestamps and `created_at` both use UTC. The filename pattern is `YYYY-MM-DD_HH-mm-ss.md` derived from the same UTC instant as the RFC 3339 `created_at` value. UTC filenames sort correctly and cannot collide or misorder across DST transitions.
- On a same-second collision, append `_01`, `_02`, and so on; never overwrite.

## 7. Vault creation and configuration

### `heimdall create` (shell only)

Vault creation and initialization is a setup operation performed by a human (directly or through the desktop app). It is a shell command only and is **not** exposed as an MCP tool.

The same rule governs every client operation (§2, §15): a shell subcommand the desktop calls, never a tool.

```text
heimdall create <name> [--root <path>] --json
```

- `--root` defaults to the process working directory.
- `name` is one folder name, not an arbitrary path.
- Existing files are never overwritten.

Behavior:

- If the target folder does not exist or is empty, write the complete template verbatim: the `aios/` structure and whatever example notes the template ships. A folder holding only filesystem debris (`.DS_Store` and friends) counts as empty.
- Whatever `docs/vault-template/` contains is what a new vault receives, so changing the example notes needs no code change. Only the managed container directories are created programmatically — `memories/extended/`, `conversations/`, `notifications/`, and `attachments/` — because an empty directory cannot be embedded or tracked in git. Filesystem debris is never copied.
- If the target folder already exists and has content (for example a folder of notes already in use), create only the missing `aios/` structure. Do not add example notes and do not touch any existing file, hidden or otherwise.
- Roll back only empty directories created by a failed operation.

The canonical template contents (the `aios/` structure and the example notes) live in `docs/vault-template/` and are embedded into the binary (for example with `include_dir`) so the CLI needs no runtime assets. The template writes no editor configuration of any kind: a vault is Markdown files, and whatever tool a user opens them with brings its own settings.

### MCP configuration

One vault per server process. The vault is fixed by server configuration; no tool call can name, switch, or discover vault paths.

```text
heimdall mcp --vault "/Users/name/Documents/My Vault"
```

MCP client configuration:

```json
{
  "mcpServers": {
    "heimdall": {
      "command": "heimdall",
      "args": ["mcp", "--vault", "/Users/name/Documents/My Vault"]
    }
  }
}
```

Rules:

- Tool calls never contain vault paths or vault IDs.
- MCP never falls back to the process working directory; `--vault` is required.
- If the configured vault is not initialized (no valid `aios/` structure), domain tools fail with `NOT_INITIALIZED` and a message telling the user to run `heimdall create` (or use the desktop app).
- To use multiple vaults, configure multiple server entries (`heimdall-personal`, `heimdall-work`), each with its own `--vault`. Native multi-vault mode is deferred (see §13).
- Client-provided MCP Roots do not expand authority in V1.

## 8. Bounded discovery and reads

No operation reads an entire vault without limits. `link_graph` (§15) scans the whole
vault and is bounded by node, per-file, and total-byte caps rather than by pagination.

### Listing limits

- Default page size: 50 entries. Maximum page size: 200 entries.
- Listings return metadata only, never file content.
- Do not hash file content during listing. Revisions are returned by reads and successful creates/writes.
- `list_documents` is non-recursive by default. Recursive listing requires an explicit flag, defaults to a depth of 4, and has a maximum depth of 16.
- Documents sort by relative path, byte-wise, over the whole result set rather than by traversal order — that total order is what makes "resume strictly after this path" well defined. Entries sort newest first, breaking ties by filename. Memories list the main memory first, then extended memories by filename.
- `list_documents` returns directories and `.md` files. Vaults may hold other files, but V1 exposes no operation that can read one, so listing them would be a dead end. Hidden entries (anything beginning with `.`), filesystem debris such as `.DS_Store`, Heimdall's write-temp sidecars, and the `.lock` sidecars older versions left beside notes are never listed.
- `list_documents` uses stateless continuation: when another page exists, the response includes `next_cursor`, which is simply the last relative path returned. The next call passes it as `cursor` and listing resumes strictly after that path under the same parameters. A malformed cursor is `INVALID_INPUT`. There is no server-side cursor state to invalidate.
- As a safety guard, one `list_documents` call resolves at most 10,000 filesystem entries; if the guard is hit, return the partial page with `next_cursor`. Only entries beyond the cursor count against the guard: ground a previous page already covered was paid for by that page, and charging for it again would make the far end of a large vault permanently unreachable. Entries a call can reject on name alone — hidden, debris, not Markdown, already behind the cursor — cost nothing.
- `list_memories` and `list_entries` do not paginate. Their folders are small and bounded in practice; they honor `limit` (default 50, maximum 200) and clients may raise `limit` when they need more.
- Listings are weakly consistent while files change; clients may re-list when they need a fresh snapshot.

### Read limits

- A `read_documents` request may select at most 10 files.
- Default range per file: 200 lines. Maximum requested range per file: 1,000 lines.
- Default total returned content: 64 KiB. Hard total returned content: 256 KiB.
- Read protected content with the same line and byte limits.
- Split only at UTF-8 and line boundaries.
- Return `complete: false` and `next_line` when more content remains.
- Never silently discard or imply that a partial file is complete.

### Link graph limits

`link_graph` is a client operation (§2, §15): it indexes the whole vault in one call,
including `aios/`, and is not an MCP tool. It returns no file content — only paths,
titles, sizes, modification times, and link endpoints, which is strictly less about the
protected tree than `list_memories` and `list_entries` already report.

Being whole-vault, it is bounded by caps rather than by continuation:

- At most 5,000 nodes. Beyond that the response sets `truncated.node_cap_hit` and counts the Markdown files it did not carry.
- At most 256 KiB scanned per file. A larger file is still a node, with `scanned: false`.
- At most 64 MiB scanned in total. When that budget is spent, the remaining nodes come back with `scanned: false` and `truncated.total_bytes_cap_hit` is true.
- At most 1,000 links extracted from any one file.

A cap is a truncated success, not a failure — the same treatment `list_documents` gives
its scan guard. `scanned: false` is the difference between "this note has no links" and
"its links were not read", and the response never conflates the two.

Edge endpoints are indices into `nodes`, not paths. That is a payload decision rather
than a stylistic one: with string endpoints a 5,000-note vault would nearly fill the
desktop bridge's stdout ceiling, and the failure would arrive as unreadable output rather
than as an honest truncation flag.

Backlinks are the reverse of `edges` and are computed by the client. Returning them as
well would duplicate every edge and let the two drift apart.

Every read result includes:

```json
{
  "path": "projects/my_project.md",
  "content": "# My Project\n",
  "start_line": 1,
  "end_line": 2,
  "next_line": null,
  "complete": true,
  "size_bytes": 13,
  "revision": "blake3:..."
}
```

## 9. V1 MCP surface

V1 is tools-only for broad client compatibility. Tool names remain snake case; shell subcommands use kebab case (`heimdall list-documents`, `heimdall read-memory`, and so on) and mirror the same core operations.

### Discovery and reads

| Tool | Use |
|---|---|
| `list_documents` | Discover ordinary Markdown notes before choosing what to read. Never use it to access `aios/`. |
| `read_documents` | Read selected, bounded ranges of ordinary notes returned by `list_documents`. |
| `list_memories` | Discover the main memory and extended memory files. |
| `read_memory` | Read a bounded range from the main memory or one selected extended memory. The main memory is where the user's standing instructions live, so read it first. |
| `list_entries` | Discover conversation summaries and notifications by metadata, filtered by kind. |
| `read_entry` | Read one selected entry. |

### Mutations

| Tool | Use |
|---|---|
| `write_memory` | Replace the main or one extended memory using an expected revision. Do not use it for ordinary notes or append-only logs. |
| `create_entry` | Create a new compressed conversation summary or notification. Never replaces an existing entry. |

Eight tools total. Do not add a general `read_file`, `write_file`, or `execute` tool. Vault creation is shell-only (§7).

There is deliberately no agent-instructions file and no tool to read one. MCP already has a channel for telling a client how to behave — the `instructions` string published at initialization (§12) — and what varies per vault is the user's own durable context, which is the main memory. A third place to say the same thing is a third place to keep in step.

The desktop's client surface is separate, and larger (§15). It is reached only through
shell subcommands, and its request and response types deliberately derive no
`JsonSchema` — `rmcp` builds a tool's schemas from those types, so a client operation
cannot be given a tool without a compile error. "Eight tools" is therefore a property the
compiler holds, not a promise review has to keep.

## 10. Tool contracts

### `list_documents`

Input:

```json
{
  "path": "projects",
  "recursive": false,
  "max_depth": null,
  "cursor": null,
  "limit": 50
}
```

Each result entry returns relative `path`, `kind` (`directory` or `document`), `size_bytes` for documents, and `modified_at`. It never returns content or a revision. The operation excludes `aios/` at every depth. Continuation follows §8.

### `read_documents`

Input:

```json
{
  "documents": [
    {
      "path": "projects/my_project.md",
      "start_line": 1,
      "max_lines": 200
    }
  ],
  "max_total_bytes": 65536
}
```

Behavior:

- Require one to ten explicit Markdown paths.
- Reject directories; callers discover files with `list_documents`.
- Return each selected range with revision and continuation metadata.
- Stop before the total byte cap and mark remaining selections as not returned.

### `list_memories`

Input:

```json
{
  "limit": 50
}
```

Returns the main memory first, followed by extended memory filenames and basic metadata. It does not return content or revisions.

### `read_memory`

Main memory input:

```json
{
  "start_line": 1,
  "max_lines": 200
}
```

Extended memory input:

```json
{
  "extended": "project_history.md",
  "start_line": 1,
  "max_lines": 200
}
```

`extended` accepts one base filename ending in `.md`; it never accepts a path.

### `write_memory`

Input:

```json
{
  "content": "# Memory\n\nCompressed durable context.",
  "extended": null,
  "expected_revision": "blake3:..."
}
```

Behavior:

- Replace the complete main memory when `extended` is null.
- Replace or create exactly one file inside `aios/memories/extended/` when `extended` is a safe Markdown filename.
- Require `expected_revision` for an existing file.
- Require explicit JSON `null` as `expected_revision` when creating a new extended memory.
- Compare the revision again while holding the vault's cross-process write lock.
- On a mismatch, return `REVISION_CONFLICT` with the current revision but not the current content.
- Write to a temporary sibling, flush, atomically rename, and return `new_revision`.
- Never truncate, merge, or summarize automatically.

Main-memory policy:

- Hard storage limit: 32 KiB (32,768 UTF-8 bytes). Writes above it fail with `LIMIT_EXCEEDED`.
- Advisory warning: any successful write of 24 KiB or more returns a warning recommending that durable topic detail move into descriptive files under `extended/`, linked from `memory.md`.
- Extended memories remain topic-specific and have a 1 MiB per-file write limit.

Both thresholds are byte-based. Token counts vary by model and tokenizer, so Heimdall does not estimate tokens.

### `list_entries`

Input:

```json
{
  "kind": "conversation",
  "limit": 50
}
```

- `kind` is required: `conversation` or `notification`.
- Returns filename (`id`), creation time, and size, newest first. Read a selected entry to obtain its revision.
- `created_at` comes from the filename, which Heimdall derives from the UTC instant of creation, so a listing costs no file reads. Entry folders are ordinary folders and may hold files a user added by hand; those still list, falling back to the filesystem modification time. Nothing a user puts in the vault becomes invisible.

### `read_entry`

Input:

```json
{
  "kind": "conversation",
  "id": "2026-08-16_10-30-00.md",
  "start_line": 1,
  "max_lines": 200
}
```

`id` is a base filename returned by `list_entries`; it never accepts a path. `kind` selects the folder and must match where the entry lives.

### `create_entry`

Input:

```json
{
  "kind": "notification",
  "content": "# Notification\n\nHigh-level compressed notification."
}
```

Behavior:

- Creates a new UTC-timestamped Markdown file in `aios/conversations/` or `aios/notifications/` according to `kind`.
- Adds Heimdall-owned frontmatter with `created_at` and `type` matching `kind`.
- Rejects content that begins with its own frontmatter block with `INVALID_INPUT` (§6).
- Never replaces an existing entry; collisions get a numeric suffix, up to `_99` for one second. Creation uses `O_EXCL`, so a collision is detected by the filesystem rather than by a check another process could race.
- Content is limited to 1 MiB; larger bodies fail with `LIMIT_EXCEEDED`. The shell applies the same bound to stdin.
- Returns the new entry's `id`, metadata, and revision.

## 11. Shell and MCP output semantics

### Shell CLI

Shell commands use a stable JSON envelope:

```json
{
  "ok": true,
  "data": {},
  "meta": { "schema_version": 1 }
}
```

```json
{
  "ok": false,
  "error": {
    "code": "NOT_FOUND",
    "message": "Document does not exist",
    "details": {}
  },
  "meta": { "schema_version": 1 }
}
```

Markdown write content is supplied through stdin or a request body, never as a shell-interpreted string.

Running `heimdall` without arguments prints help and performs no mutation.

Exit codes:

- `0` — `ok: true`.
- `1` — an expected domain failure. The error envelope is still printed on stdout; only the status differs, so a script can branch without parsing and a caller that does parse loses nothing.
- `2` — a usage error from argument parsing. The request never reached the domain, so there is no envelope.

Every domain subcommand requires `--vault <path>`, with no working-directory fallback, matching the rule the MCP server follows (§7): the vault a command operates on is always explicit. `heimdall create` instead takes `--root`, which defaults to the working directory, because it is naming a location rather than selecting an existing vault.

`write_memory`'s `expected_revision` distinguishes an absent field from an explicit `null`: `null` means "this file does not exist yet", while omitting it means the caller forgot the revision it read, and treating that as a create would let a stale client clobber a file. The shell spells the `null` case as `--create`.

### MCP

Do not copy the shell envelope into MCP.

- Define an `outputSchema` for every tool.
- Return the typed result directly in `structuredContent`.
- Include a concise text fallback for clients that do not render structured content.
- Represent expected domain failures as a tool result with `isError: true` and structured `code`, `message`, and safe `details`.
- Reserve protocol-level errors for unknown tools, failed initialization, or calls the server cannot execute.
- Arguments that violate a tool's input schema are a client mistake the server understood, so they come back as `INVALID_INPUT` in the same structured shape rather than as bare text. Every failure a client can cause carries a code it can branch on, and the SDK's own argument-validation results are normalized to match.
- Never expose stack traces or unnecessary absolute paths.

Domain codes:

- `INVALID_INPUT`
- `LIMIT_EXCEEDED`
- `PATH_OUTSIDE_VAULT`
- `NOT_FOUND`
- `ALREADY_EXISTS`
- `NOT_INITIALIZED`
- `REVISION_CONFLICT`
- `IO_ERROR`
- `INTERNAL_ERROR`

## 12. MCP server behavior

Initial transport: stdio.

Requirements:

- Log diagnostics only to stderr because stdout belongs to MCP.
- Generate tool input/output schemas from shared Rust types where practical. The `heimdall-core` request and response types derive `JsonSchema`, so every tool's `inputSchema` and `outputSchema` follow the domain types automatically and cannot drift from what the shell returns.
- Pin `rmcp` and an MCP protocol baseline; upgrade deliberately after contract and conformance tests pass. Current pins: `rmcp` `=3.1.3`, protocol `2025-11-25`.
- Bound how long any operation can wait. Reads and listings are bounded by the limits in §8, and waiting for a contended write lock has its own ceiling (§14) so one wedged process cannot hang the server.
- Enforce the configured vault boundary before dispatching to core operations.
- Support cancellation and bound operation duration.
- Support `heimdall --version --json` with CLI version, core version, MCP protocol compatibility, and output schema version.

### Server instructions

The `instructions` string returned from `initialize` is how a server tells a client how
to work with it, and it is the only such channel Heimdall has: there is no instructions
file for a client to go and find, because a client that had to call a tool to learn the
rules would already have made its first call without them.

Publish concise server-level instructions equivalent to:

> Heimdall manages one Markdown vault. Ordinary notes live outside `aios/`; protected memories and entries (conversation summaries and notifications) live inside it. Read the main memory with `read_memory` at the start of a session: it holds the user's durable context and how they want you to work in this vault. List content before reading it, request only the ranges needed, and follow continuation metadata. Use `write_memory` only for durable memory and always pass the revision returned by the latest read. Use `create_entry` for conversation summaries and notifications; it never replaces existing files.

What is fixed for every vault belongs in that string; what varies per vault belongs in
the main memory, which the string points at. Every tool also receives a strong
description explaining when to use it, what it cannot access, its limits, and whether it
mutates content.

## 13. Roadmap (deferred by design)

Deferred features must preserve the same vault restrictions, limits, revisions, and continuation behavior.

- **MCP resources.** Read-oriented content is semantically suitable for resources later (`heimdall://documents/<relative-path>`, `heimdall://memories/main`, and so on). Adding resources does not remove the read tools; creation and mutation remain tools.
- **Native multi-vault mode.** Friendly vault IDs on every call plus a `list_vaults` tool. Until then, one server process per vault (§7).
- **MCP Roots** as an explicit, reviewed configuration source.
- **Attachment operations**, only with explicit MIME, size, and security rules.

## 14. Filesystem safety and concurrency

- Open the configured vault as a directory capability and resolve descendants beneath it.
- Do not rely on string-prefix path checks.
- Canonicalize existing roots and parents.
- Reject absolute child paths, `..` traversal, NUL bytes, platform prefixes, and symlinks escaping the vault.
- Validate a new destination beneath an already validated parent.
- Sort reads and listings deterministically.
- Use temporary sibling files, flush them, and atomically rename.
- Hold one cross-process write lock (`fs4`) per vault around revision comparison and replacement. Optimistic concurrency only holds if comparing and replacing are one indivisible step; without it two writers both read revision A, both find it current, and both write, and one edit is lost with no error.
- The lock lives **outside the vault**, in the per-user application-data directory, in a file named by a hash of the vault's canonical path. It cannot live on the target, because replacing a file renames a new inode over it and each writer would hold a lock on a different file. It must not live on a sidecar beside the target either: a sidecar can never be safely removed — unlinking one another process is about to open leaves the two locking different inodes — so every note ever written left a `.lock` file in the user's vault. A vault holds the user's Markdown and nothing of Heimdall's.
- Application data, not a cache directory: a cache is something the system may purge, and a purge that unlinks a held lock file is the same divergence that makes lock files unsafe to delete. `HEIMDALL_LOCK_DIR` overrides the location for a deployment with no writable home.
- The vault root is canonicalized when the vault is opened, because the lock key is what two processes must agree on and a path as typed is not that — `~/vault`, `./vault`, and a symlink to it are three spellings of one directory that would otherwise take three separate locks and exclude nothing.
- One lock per vault is coarser than one per file and therefore strictly stronger, so it cannot introduce a race. A holder only hashes some bytes and renames a temp file, so serialising a vault's writes costs nothing at the rate they arrive. It does mean a locking operation must never be called from inside another one: with a single lock that is a self-deadlock rather than merely redundant.
- Write temps are the one thing Heimdall does put in a vault, and unavoidably: an atomic write lands in a temporary sibling and is renamed into place, and a rename cannot cross filesystems. It is removed on every path including failure, so one survives only a kill or a power loss.
- Bound the wait for a contended lock (10 seconds) rather than blocking indefinitely. Every holder does short, bounded work, so exceeding that means another process is stuck, and reporting `IO_ERROR` beats inheriting its hang — a server cannot promise a bounded operation duration on top of an unbounded wait.
- A rename never replaces an existing destination. `cap-std` exposes no `RENAME_NOREPLACE`, so the check and the rename both happen under the vault's write lock, and a collision is `ALREADY_EXISTS`. This serialises Heimdall processes against each other, not against another editor writing into the same vault — the same weak consistency §8 already accepts for listings.
- Deletion moves content into `.trash/`; nothing is ever unlinked. Collisions there take numeric suffixes, exactly as entry filenames do.
- Compute revisions from exact stored bytes after a successful write.
- Apply read-size, file-count, stdin-size, and execution-time limits.
- Redact sensitive path segments and note content from production logs.

## 15. Desktop application

The desktop application is where a human works on their vault, and where the local MCP
server is configured. It is a three-pane workspace — file tree, Markdown editor with
preview, link graph — with setup, server configuration, and diagnostics behind a Settings
modal reached from the application menu.

No other editor is required, and none is displaced: the vault is plain Markdown, so
Heimdall and any other Markdown editor can be used over the same files.

### The workspace

1. **Files** — the whole vault, `aios/` included. Ordinary folders come from `list_documents`; the protected tree can only come from `link_graph`, because ordinary listing excludes `aios/` at every depth by design (§6). New note, new folder, sort order, and collapse-all sit in a toolbar above the tree. A right-click offers rename and delete for ordinary content only: both operations refuse `aios/`, so offering them there would produce nothing but an error. A rename that will break inbound links says how many before it happens, and a delete says where the note is going.
2. **Note** — a breadcrumb with back/forward history, a source/preview toggle, and an overflow menu. The heading is the filename: there is no separate title to keep in step, so editing the heading renames the file and renaming the file changes the heading. Preview renders the note, shows its YAML frontmatter as a properties table with clickable wikilinks, renders fenced `mermaid` diagrams, and lists linked mentions — the notes that link to this one, each a name to click beside its path and nothing else. The note's own outgoing links are not repeated under it: they are in its text a few lines above, and a second copy is one more list to read past. Source is a Markdown editor with the frontmatter block and heading markers dimmed.
3. **Graph** — a force-directed graph of the whole vault, built to match what a vault graph view does. Pan (with inertia), zoom about the pointer, drag a node and watch its neighbours follow, click one to open it. The note being read is drawn in the accent colour; hovering one lights its links and dims everything more than a step away.

   Four details carry most of the resemblance, and each is easy to get wrong:

   - **Node and label size scale with the square root of the zoom**, not with the zoom. The container scales by the zoom while every node and label counter-scales by `1/√zoom`; that half-rate growth is the signature of how the graph feels.
   - **Link thickness is constant on screen** at every zoom.
   - **Labels fade in over one octave of zoom** — invisible below `2^(t-1)`, opaque at `2^t`, where `t` is the text fade threshold.
   - **Everything transitions** rather than switching: dimming and colours lerp a tenth of the way per frame.

   The forces are a vault graph view's, translated into d3's units and scaled for a side pane; note that the pull toward a point is `forceX`/`forceY` rather than `forceCenter`, which is a hard recentring translation that would make the graph impossible to drag off-centre. Where such a view stores its settings it stores *slider positions*, not force values, which is why feeding them to d3 directly throws the layout several pane-widths across.

   The view is fitted to the graph when the layout settles, and left alone once the user has panned or zoomed themselves. The fit fills 88% of the pane along whichever axis is tighter — not a fixed padding and a magnification cap, which for an ordinary vault left the graph two thirds of the way across a side pane and adrift in it — with a floor on the extent it will magnify, so two linked notes are not blown up into two dots the size of coins. What is measured is the node positions: a long label on an outermost node can run past the edge, and fitting the labels as well collapses the fit back to roughly where it was — in a side pane the longest title decides everything. Node and label sizes follow the fit through the square-root law above, and the label size is set so that at the default fit it reads at the same 12px as the application's other small text. Refitting is flown rather than assigned — the layout settles a second or two after a node is dragged, and moving the camera outright at that moment reads as the graph skipping. Recentre travels the same way, and touching the graph stops the travel. Node positions survive a re-index, so saving a note does not scatter the layout. Whatever the index had to leave out is stated in the pane; a graph that quietly showed most of a vault would make real links look broken.

The panes open as proportions of the window — roughly 14% files, 46% note, 40% graph — rather than at fixed pixel widths, because the window opens filling the display. A divider travels until the note reaches its own minimum width given the pane on the other side, not until a fixed share of the window, and a note narrower than its content scrolls horizontally rather than reflowing into an unreadable column.

A quick switcher (⌘O) opens a note by name.

### Settings

Reached from **Heimdall → Settings…** (⌘,) in the application menu, as a modal over the
workspace rather than a screen inside it — these are occasional tasks, and the window
belongs to the notes. It has four sections:

1. **Vault** — create a new templated vault (`heimdall create`) or select an existing folder of notes and initialize its `aios/` structure.
2. **Server** — make the MCP server available: show the exact `heimdall mcp --vault ...` command, generate the `mcpServers` JSON snippet, and (with explicit user consent) write it into known client configuration files. Writing merges into the existing configuration rather than replacing it, backs the previous file up first, and saves through a temporary sibling so an interrupted write cannot leave a client with half a file. A second vault gets its own entry name instead of taking over the first one's. A development build refuses to write at all, and the screen says so before the click: its sidecar is a build artifact that a rebuild or `cargo clean` removes, and a client whose configured command has gone reports a timeout rather than a missing file — so the entry would fail silently and much later. For the same reason the screen reports a registration that is already stale: an entry for this vault whose absolute command is no longer on disk. Provide a one-click health check that launches the server, performs an MCP handshake, and reports the result.
3. **Appearance** — System, Light, or Dark, and an accent colour for each theme. System is the default and is applied by the stylesheet's media query, so the first paint is correct without waiting for JavaScript. The accent is offered as a colour well beside the hex it resolves to, with a control that clears the choice rather than writing the default back; light mode has no hue to set.
4. **Diagnostics** — active CLI path and versions, protocol/schema compatibility, configured vault path and initialization status, limits, and recent actionable errors.

The properties table is shown only for a note that actually has a `---` block; a
heading over an empty grid is furniture rather than information. Adding one is
offered from the note's overflow menu, asks for a name and a value together, and
creates the block when there was not one.

Notes and folders can be dragged onto a folder in the tree to move them, or onto
the tree itself — the empty space below the rows — to move them to the vault
root, which is the way back out of a folder. Both go through the same
`move_path` as the rename. The drag is a pointer-event one: WebKit's own drag is
turned off on the rows, since it takes the press and then cancels the gesture.

Installing a custom application menu replaces the platform default, so the menu must also
carry the predefined Edit items. Without them the standard Cut/Copy/Paste/Undo
accelerators stop reaching the editor.

### Client operations

Editing needs writes that the MCP surface does not have and must not grow (§9). These are
shell subcommands the desktop calls, exactly as `heimdall create` is (§7):

| Command | Use |
|---|---|
| `write_document` | Replace or create one ordinary note, guarded by `expected_revision`. |
| `create_folder` | Make a folder for ordinary notes. |
| `move_path` | Rename or move a note or folder. Refuses to cross the `aios/` boundary in either direction, and refuses the managed structure. |
| `delete_path` | Move a note or folder into the vault's `.trash/`. Nothing is ever unlinked. |
| `write_entry` | Replace one entry, preserving Heimdall's `created_at` and `type` (§6). |
| `link_graph` | Index the whole vault's links in one call (§8). |

Rules:

- None of these is an MCP tool, and none of their types derives `JsonSchema`. The tool surface stays at exactly eight (§9), and a compile error is what enforces it.
- Writes are optimistically concurrent. `expected_revision` is required; explicit `null` means "create", and omitting it is `INVALID_INPUT` rather than a silent create.
- A stale revision returns `REVISION_CONFLICT` with the current revision in `details`, and the client resolves it by asking the user — never by merging and never by overwriting.
- Renaming a note does **not** rewrite `[[wikilinks]]` in other notes. That would be an unbounded multi-file write with no revision check on any of the files it touched. The client warns about inbound links instead, which it can do because the index already knows them.
- `link_graph` is a load-and-refresh operation, not an interactive one: it reads every note in the vault. Call it when a vault is opened and after a structural change, never per keystroke.

### Reading a whole note

Reads are bounded (§8), so the editor follows `next_line` until `complete`. The revision
each chunk reports covers the entire file at that moment — `read_range` hashes all of the
bytes before slicing out the requested lines — so a file that changes mid-read is always
visible as a revision that stopped matching, and the read restarts rather than stitching
two versions together. The window between the last chunk and the first write is closed by
`expected_revision`, not by the read.

### Visual design

- Dark mode: pure black background (`#000`), white text (`#fff`). Light mode: white background, black text. The system preference is the default, with an explicit override in Settings.
- **One accent per theme**, both the user's, set in Settings → Appearance. Not one accent shared between them: no single colour works on both grounds, since white is the right accent on black and invisible on white and black is the reverse, so a shared accent makes every choice a compromise and the obvious ones unusable in one theme. Each is used as picked, neither lightened nor darkened, and the Appearance copy says to choose one that reads against its own background. The accent is spent on links, the row of the note you have open in the file tree, the active graph node, and mermaid diagrams, and nothing else. The open note is *named* in the accent rather than sat on a block of grey: a selected-row highlight says "list box", and nothing else in this application talks that way.
- Until an accent is chosen that theme is monochrome, falling back to its own end of the palette — black links on white, white on black. So each choice is a never-declared custom property, `--accent-light` and `--accent-dark`, written inline on the root element by Settings, and each theme resolves `--accent` from its own with a declared `--accent-*-base` as the `var()` fallback. An unset custom property falls through to that fallback, which is what makes a choice clearable and keeps theme resolution out of TypeScript entirely — each theme reads only its own property, so writing either is safe whatever is showing. Both bases are declared on `:root` rather than inside their theme blocks for two reasons: Settings shows a well per theme and must read the default for the theme that is *not* showing, and reading a declared property does not depend on how a browser computes a `var()` chain.
- A preference stored before the accent was split is a single value, and is read back as dark mode's — what it originally meant. Losing someone's colour because the shape around it changed is a poor trade for a few lines.
- The accent is reached everywhere else through a custom property, so the CodeMirror theme and the canvas renderer read it from CSS rather than naming a colour — and anything drawn rather than styled, the graph and mermaid, must redraw when the choice changes.
- Every border in the application is 1px solid, squared (`border-radius: 0`), and drawn in one colour — the same quiet grey as the pane dividers. Borders are structure, not emphasis; at full contrast every panel, input and dialog shouts.
- Nothing is selectable but the note. The editor, the preview, the fields you type into and the setup commands that exist to be copied opt back in; every other run of text in the application — filenames, breadcrumbs, headings, labels — is furniture, and a drag across it should not paint it grey.
- The pointer says what a thing is. Anything that can be pressed — every button, link and mention in the application — shows the hand; a control that is disabled shows that it will not answer. The rule is stated once, on the element rather than on each class, so a control added later is already covered and cannot end up the one arrow on the screen. Field labels are controls too — clicking one focuses its field — and have to be named explicitly, because WebKit's own stylesheet hands them an arrow. The pane dividers show a resize cursor and the graph canvas a grab; those are the only exceptions.
- Hovering a pane divider does not light it up. The resize cursor already says the rule can be dragged, and it says it from the whole seven-pixel track rather than from the one pixel that would change colour; brightening the hairline as well flashed two lines across the window on every pass of the mouse. Keyboard focus still shows, because a divider reached with Tab has no pointer over it to say where it is.
- The window draws no title bar of its own. The webview runs under an overlaid one (`titleBarStyle: Overlay`), so the pane dividers reach the top of the window as they already reach the bottom, and the file pane's toolbar and the note's header sit in that band beside the platform's own window controls rather than below them. What the controls take out of the top-left corner is stated once as a token and left to whichever surface is there — the toolbar normally, the note's header when the file pane is put away, a banner while one is up. A divider that would otherwise run under those controls starts below them instead: a hairline drawn through the close button is one nobody can grab. Moving the window is then the application's own job, so the same surfaces are drag regions, and the capability that permits it is declared like any other.
- The window opens filling the display. A vault, a note and a graph side by side is what the workspace is for, and a window that starts at two thirds of the screen makes the first thing anyone does resizing it.
- Typography: Helvetica via a system-safe stack — `"Helvetica Neue", Helvetica, Arial, sans-serif`.
- Markdown is rendered by walking the lexer's tokens into elements, never by building an HTML string. Agents write into this vault, so a note containing markup is ordinary input; not producing HTML removes the class of problem rather than sanitising it afterwards.

### CLI bridge

Expose a small Tauri command surface:

```text
cli_status() -> CliStatus
invoke_cli(command, request, stdin?) -> CliResponse
```

Rules:

- The desktop always uses its bundled sidecar CLI; it never searches `PATH` or executes a user-supplied binary. The sidecar resolves to an absolute path beside the app's own executable, falling back to the staged development copy — never to a bare name a `PATH` lookup could satisfy.
- Allowlist command names **and each command's argument keys**; never accept an arbitrary executable or raw shell string. Values become separate `argv` entries rather than being interpolated, so a vault path containing spaces, quotes, or a semicolon is only ever a path.
- `mcp` is not an allowlisted command: starting a long-running server is the client's job, and the health check has its own path.
- Keep process execution in Rust; grant React no general shell capability.
- Spawn the CLI without a shell.
- Pass Markdown through stdin.
- Parse versioned JSON and return typed data to React.
- Set timeouts and support cancellation.
- Capture bounded stderr for diagnostics.
- Serialize conflicting writes to the same destination.

Start with one CLI process per user action. Add a persistent process only if startup latency becomes a measured problem.

## 16. Installation and distribution

### Headless CLI

The headless artifact contains only the Rust CLI/MCP executable and notices.

Installation options may include:

- `cargo install heimdall-cli`
- A platform package manager or release archive
- A signed standalone `heimdall` binary

Installing the CLI must not install Node.js, React, WebView assets, or the desktop application.

### Desktop

Every desktop release bundles a signed, version-matched `heimdall` CLI as a Tauri `externalBin` sidecar. Build artifacts use Tauri's required target-triple suffix, such as `heimdall-aarch64-apple-darwin`.

The desktop always invokes its own bundled sidecar. It performs no CLI discovery, no compatibility negotiation with independently installed CLIs, no managed side-by-side copies, and no shims. A separately installed CLI simply coexists; Diagnostics shows which binary the desktop is using.

- Desktop upgrades replace the bundled sidecar as part of the normal app upgrade.
- The MCP config snippet generated by the Server screen references the sidecar's absolute path (or the user's own `heimdall` if they prefer and say so). Writing that path into a client's configuration is offered only by a shipped build (§15).
- Never require administrator privileges for normal per-user installation.

## 17. Testing strategy

### Core tests

- `heimdall create` writes the full template (`aios/` and the template's example notes) into an empty or missing target, and copies no filesystem debris
- `heimdall create` on a folder that already has content adds only missing `aios/` structure, never touches an existing file (hidden ones included), and adds no example notes
- `aios/` exclusion from ordinary discovery and reads
- Listing pagination via `cursor`/`next_cursor` continuation, deterministic ordering, and the entry-scan guard
- Multi-file selection, line continuation, and byte/file caps
- Protected memory and entry listing/reads for both kinds
- UTF-8 boundaries and invalid UTF-8 errors
- Traversal and symlink escape rejection
- UTC timestamp collision handling (numeric suffixes)
- Frontmatter ownership: `create_entry` rejects content beginning with a frontmatter block
- Atomic writes and interrupted-write recovery
- Main-memory 32 KiB boundary and 24 KiB advisory warning
- Revision match, conflict, new-file null revision, and cross-process locking

### CLI/MCP contract tests

- Shell and MCP adapters produce equivalent domain outcomes.
- Shell output retains its envelope.
- MCP output uses typed `structuredContent` without the shell envelope.
- Expected domain failures and protocol failures are distinguished.
- stdout contains protocol/JSON only; stderr cannot corrupt it.
- The configured vault boundary is enforced; no tool call can supply a vault path.
- An uninitialized vault yields `NOT_INITIALIZED` with actionable guidance.

### Client operation tests

- Saving a note requires the revision it was read at; a stale one is `REVISION_CONFLICT` and the file is untouched.
- Omitting `expected_revision` is a caller mistake rather than a create.
- Document writes cannot reach `aios/` in any casing, or any hidden folder.
- Deletion lands in `.trash/`, keeps the original layout, takes a numeric suffix on collision, and disappears from listings.
- The managed structure cannot be moved or deleted, and the vault still initializes afterwards.
- A move refuses to cross the `aios/` boundary in either direction and refuses to leave Markdown behind.
- An entry edit preserves `created_at` and `type`, accepts the user's own keys, and cannot move an entry between kinds.
- Link resolution follows the conventional wikilink rules, including the shortest-path tie-break, and is byte-for-byte deterministic across runs.
- Links inside fenced code, inline code, and unterminated frontmatter produce no edges; links inside real frontmatter do.
- The graph covers `aios/`, marks those nodes, carries no file content, and truncates at each cap while reporting what it left out.
- The MCP server still advertises exactly eight tools, and none of the client operations appears among them — checked both in-process and over a real stdio connection.

### Desktop tests

- The bundled sidecar is invoked (never a `PATH` binary), without a shell, with Markdown over stdin.
- Setup creates a templated vault and initializes an existing vault correctly.
- The Server screen generates a valid client config and the health check completes an MCP handshake.
- Structured failures remain actionable and do not crash the UI.
- Theme, border, and typography rules render correctly in dark and light modes, and each theme's accent reaches the page only through a custom property. One theme's accent is never the other's.
- No colour literal appears anywhere in the TypeScript source; the editor theme and the graph renderer read the stylesheet.
- A whole note is assembled from bounded reads, restarts when the file changes mid-read, and compares bytes rather than characters when checking the result.
- The file tree shows the protected tree, which only the link index can supply.
- Frontmatter round-trips without reformatting the keys the user did not touch.
- Markdown renders as elements: raw HTML in a note is shown as text, never mounted.
- Settings opens from the application menu event and closes from both the X and Escape.

### Release smoke tests

For every supported operating system:

1. Install only the CLI and exercise all headless operations, including `heimdall create`.
2. Install the desktop app on a clean user account; create a vault, run the health check, and install a client config.
3. Open an existing folder of notes and verify that its files, hidden ones included, remain unchanged after initialization.
4. Upgrade and uninstall; preserve all vault content.

## 18. Delivery phases

### Phase 1 — Core and direct CLI

- Build the Cargo workspace and typed request/response models.
- Implement vault capabilities, limits, revisions, locks, and atomic storage.
- Implement `heimdall create` with the embedded vault template.
- Implement all discovery, read, and mutation operations.
- Implement shell commands and JSON envelopes.

### Phase 2 — MCP

- Add the stdio adapter and server instructions.
- Publish the V1 tool schemas and descriptions.
- Add MCP contract and boundary tests.

### Phase 3 — Desktop

- Scaffold Tauri 2 with React, TypeScript, and Vite.
- Implement the sidecar bridge.
- Build the three screens and the visual design system.
- Add desktop integration tests.

### Phase 4 — Packaging

- Produce signed CLI artifacts and desktop installers.
- Bundle the correct CLI sidecar for each target triple.
- Verify clean installation, upgrades, and uninstallation.

### Phase 5 — Desktop editor

Delivered ahead of Phase 4, which does not block it.

- Add the client write operations and `.trash/`.
- Add the link graph and its bounds.
- Build the three-pane workspace, the Settings modal, and the application menu.
- Leave the MCP surface unchanged.

## 19. Definition of done

- The installed executable and MCP command are named `heimdall`.
- The CLI works without the desktop application.
- `heimdall create` produces the complete templated vault and safely initializes an existing folder of notes.
- The desktop uses its bundled CLI for every domain operation and can install a working MCP client configuration.
- No operation performs an unbounded vault read.
- Ordinary reads cannot expose `aios/`.
- Memories and both entry kinds can all be listed and read.
- MCP results use typed structured content rather than the shell envelope.
- The MCP surface has no vault-path or vault-creation capability.
- Stale complete-file memory writes fail with `REVISION_CONFLICT`.
- The main memory respects the stable 32 KiB limit with a byte-based advisory warning.
- No write can escape the vault or silently replace unrelated content.
- The MCP surface is still exactly eight tools after the desktop editor ships, and no client operation is reachable through it.
- No deletion unlinks user data.
- Vault data survives CLI/Desktop upgrades and uninstallation.

## 20. Decisions retained from the original requirements

- The protected on-disk folder remains named `aios/`.
- `write_memory` replaces a complete target; it does not patch or append.
- Entry names begin with `YYYY-MM-DD_HH-mm-ss` (now derived from UTC rather than local time).
- Generated summaries remain high-level and compressed.
- Vault may contain non-Markdown files, but V1 exposes no attachment operation.
- Conversations and notifications keep separate on-disk folders even though the tool surface is unified.

## 21. Verified implementation references

Verified on 2026-08-16:

- [Tauri 2: Embedding External Binaries](https://v2.tauri.app/develop/sidecar/) — `externalBin`, target-triple naming, sidecar execution, and scoped permissions.
- [Tauri 2: Distribution](https://v2.tauri.app/distribute/) — platform packages, signing, and notarization.
- [Official MCP Rust SDK](https://github.com/modelcontextprotocol/rust-sdk) — the `rmcp` SDK and standard local stdio transport.
- [MCP Rust SDK roadmap](https://github.com/modelcontextprotocol/rust-sdk/blob/main/ROADMAP.md) — protocol and conformance status.

At the verification date, the newest protocol support in the official Rust SDK was still progressing through Tier 1/conformance work. Pin a known-good `rmcp` release and protocol baseline instead of automatically tracking the newest revision.
