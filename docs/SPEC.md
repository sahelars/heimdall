# Heimdall Application Specification

> Product name and installed executable: `heimdall`.
> This document is the source of truth for product behavior. `AGENTS.md` summarizes working conventions and links here.

## 1. Product definition

Heimdall is an intent-aware MCP layer for Markdown vaults. It gives AI clients two verbs over a vault — `read` and `write` — instead of unrestricted filesystem access, and it gives people a guardrail over what those verbs may touch: any note, any folder, or the whole vault can be **locked**, and a locked path is read-only for everyone until the person unlocks it. Agents can see a lock but never set or lift one: locks are set from the shell and the desktop, not over MCP.

Heimdall has two independently installable products:

1. **Heimdall CLI** — a headless Rust executable providing direct shell commands and an MCP server. The CLI is the product core; it is fully usable without the desktop application. Run from inside a vault, `heimdall read` and `heimdall write` need no configuration at all.
2. **Heimdall Desktop** — a Tauri application with a React, TypeScript, and Vite interface. It both configures the server and works the vault: a file tree, a Markdown editor with preview, and an interactive link graph, alongside lock controls, vault creation, MCP client configuration, and diagnostics. A vault is a folder of plain Markdown files and nothing else, so any Markdown editor remains a perfectly good way to work over the same files.

The desktop application bundles a version-matched CLI and calls it for every domain operation.

## 2. Terminology

- **Vault** — a folder of Markdown files. It is the security and content boundary for a Heimdall session. Every Markdown file in it that is not hidden is a **note**, and every folder a **folder**; there is no protected or managed tree.
- **Revision** — a BLAKE3 hash of the exact file bytes, formatted as `blake3:<lowercase-hex>`.
- **Lock** — a rule making a note or folder read-only (§6). Locks are Heimdall's state about a vault, stored outside it (§14); they are not file permissions and do not stop another editor.
- **Registry** — the list of vault roots Heimdall knows, kept in the application-data directory, so the shell can find a vault from any folder inside it (§7).
- **Client operation** — a domain operation the desktop calls directly through the shell CLI and the MCP surface does not expose. `heimdall create` (§7) is the original; the editor's structural writes — create a folder, move, relink, delete — and `link_graph` are the rest (§15). The distinction is enforced by the type system, not by convention: a client operation's request and response types derive no `JsonSchema`, and `rmcp` cannot build a tool without one.

"Application" refers only to the Heimdall desktop product. User content locations are called vaults.

## 3. Design principles

- Keep one implementation of domain and filesystem behavior in `heimdall-core`.
- Give AI clients purpose-aware operations, not arbitrary file access: two verbs, `read` and `write`, and nothing that changes what those verbs may touch.
- Read before writing, and read only selected, bounded content.
- A lock is the user's decision and binds every writer — agents, scripts, and the desktop editor alike. Only people set and lift locks; no MCP tool can.
- One vault per server process; scope is fixed by configuration, never by tool input.
- Never overwrite user data silently.
- Use optimistic concurrency for replace-style writes.
- Prefer stateless continuation over server-side cursor state.
- Use structured, versioned output at process boundaries.
- Default to local-only operation; no network or telemetry is required.
- Nothing of Heimdall's is written into a vault.
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
- `heimdall-cli` translates shell commands and MCP calls into core operations. The shell's own conveniences — finding the vault from the working directory and resolving paths typed relative to it — happen here, before the call reaches core.
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
│   │       ├── appdata.rs
│   │       ├── errors.rs
│   │       ├── limits.rs
│   │       ├── notelocks.rs
│   │       ├── paths.rs
│   │       ├── registry.rs
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
manifest. `crates/heimdall-core/tests/` covers discovery bounds, durability,
concurrency, locks, and the vault registry; `crates/heimdall-cli/tests/` covers the output envelope,
cross-process behavior, and the shell/MCP contract, with shared helpers in
`tests/common/`. Fixtures live beside the tests that use them.

The contract tests drive a real `heimdall mcp` process over stdio with a real
MCP client, so anything that corrupted the protocol stream shows up as a failed
handshake rather than a passing assertion.

`storage.rs` owns the vault capability itself (`Vault`) alongside atomic writes
and the write lock; there is no separate vault module. `appdata.rs` locates the
per-user application-data directory and reads and writes Heimdall's own JSON
there; `notelocks.rs` holds the lock rules and `registry.rs` the list of known
vaults, both kept in that directory (§14). `timestamps.rs` holds UTC formatting.
`commands/links.rs` holds the link scanner, a `pub(crate)` helper in the same
shape as `read_range.rs` — pure string in, structure out, so the rules about
fenced code can be pinned down without building a vault. `commands/listing.rs`
is the folder half of `read`.

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

- Rust: `clap`, `serde`, `serde_json`, `thiserror`, `camino`, `cap-std`, `blake3`, `dirs`, `fs4`, `include_dir`, `time`, and `tempfile`. (`fs4` is the maintained fork of the unmaintained `fs2`.) `dirs` locates the per-user application-data directory the write lock, lock rules, and registry live in (§14). `time` supplies UTC timestamp formatting.
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
├── <folders>/
│   ├── *.md
│   └── <arbitrary files>
└── .trash/                     # deleted content; never listed, never read
```

Rules:

- Notes are UTF-8 Markdown with the `.md` extension. A vault may contain other files; Heimdall lists and reads only folders and `.md` files.
- There is no protected or managed tree. Every visible note and folder is reachable by `read` and `write`, subject to locks.
- Hidden entries (anything beginning with `.`) belong to other tools — or, for `.trash/`, to nobody — and are never listed, read, or written.
- Vault creation and registration preserve every existing file, including hidden configuration folders belonging to other tools.
- Nothing of Heimdall's is written into a vault: the write lock, the lock rules, and the registry all live in the per-user application-data directory (§14). The one unavoidable exception is an atomic write's temporary sibling, which exists for the length of one rename.
- Deleted content moves to `.trash/` at the vault root, mirroring its original folders. The leading dot means it is already excluded from every listing, and mirroring is the only layout in which restoring a file is unambiguous. Emptying the trash is not a Heimdall operation.
- Links use wikilinks or relative Markdown links, for example `[[projects/my_project|My project]]`.

### Locks

A lock makes part of a vault read-only. It is the guardrail a person puts around notes an agent — or anyone — must not change.

- **Targets.** The vault root (the whole vault), any folder, or any note. The target must exist.
- **Rules.** Locks are stored as rules, each a path marked *locked* or *unlocked*. A path's effective state is its **nearest rule**, looking at the path itself and then each folder above it up to the root; with no rule anywhere it is unlocked. So locking a folder locks everything in it at every depth, and a single note can still be unlocked inside a locked folder — or locked inside an unlocked one.
- **Folder operations mean the whole folder.** Locking or unlocking a folder (or the root) sets its rule and clears every rule beneath it. "Lock this folder" means every note in it, not every note but the ones someone once unlocked. A redundant rule — one that says what its folder already says — is never stored.
- **What a lock forbids.** Writing a locked note; creating a note or folder directly inside a locked folder; moving or deleting a locked path, a folder with anything locked inside it, anything directly inside a locked folder, or anything *into* a locked folder. A lock fixes a note's place as well as its bytes. Each refusal is `LOCKED`, with `details.path` and `details.locked_at` — the rule responsible, `""` for the vault root.
- **What a lock allows.** Reading. Every `read` result reports `locked` (and `locked_at`) for the path read, and every listing entry reports `locked`, so a caller learns before it writes that it cannot.
- **Who it binds.** Everyone who writes through Heimdall: MCP clients, the shell, and the desktop editor, which opens a locked note read-only. It is not a file permission, and another editor writing the files directly is outside it — the same boundary §14 draws for the write lock.
- **Links.** `relink` (§15) never rewrites a locked note; it reports the note instead, because a lock is exactly a promise that the note will not change.
- **Housekeeping.** Rules for paths that no longer exist — renamed or deleted outside Heimdall — are ignored and pruned on the next `lock` or `unlock`. A note moved by `move_path` cannot carry a lock, since a locked note does not move; one deleted to the trash does not take a rule with it, and a note restored from the trash comes back writable.
- **Who may lock and unlock.** People, through the shell and the desktop. Neither is an MCP tool (§9), in either direction: an agent that could `unlock` could get past any lock, and one that could `lock` a folder would erase every note-level unlock the user had set beneath it, with no way to put them back. So against an agent whose only access to the vault is Heimdall's MCP server, a lock is a boundary rather than a request. It is not a boundary against a process holding the user's own shell or filesystem access — one that can run `heimdall unlock`, rewrite the lock rules in the application-data directory (§14), or simply edit the file. That is what operating-system permissions are for, and Heimdall does not pretend otherwise.

## 7. Vault creation and configuration

### `heimdall create` (shell only)

Vault creation is a setup operation performed by a human (directly or through the desktop app). It is a shell command only and is **not** exposed as an MCP tool.

The same rule governs every client operation (§2, §15): a shell subcommand the desktop calls, never a tool.

```text
heimdall create <name> [--root <path>] --json
```

- `--root` defaults to the process working directory.
- `name` is one folder name, not an arbitrary path.
- Existing files are never overwritten.

Behavior:

- If the target folder does not exist or is empty, write the template verbatim: whatever example notes `docs/vault-template/` ships (`mode: "scaffolded"`). A folder holding only filesystem debris (`.DS_Store` and friends) counts as empty. Filesystem debris is never copied.
- If the target folder already exists and has content (a folder of notes already in use), write nothing into it at all (`mode: "registered"`).
- Either way, register the vault (below).
- Roll back only empty directories created by a failed operation.

The canonical template lives in `docs/vault-template/` and is embedded into the binary (with `include_dir`) so the CLI needs no runtime assets; changing the example notes needs no code change. The template writes no editor configuration of any kind: a vault is Markdown files, and whatever tool a user opens them with brings its own settings.

### Finding the vault from the shell

A vault is a plain folder, so nothing inside it can mark its root. The **registry** does instead: a list of canonical vault roots in the application-data directory (§14).

- `heimdall create` registers the vault it creates or adopts. So does every command given an explicit `--vault` — including `heimdall mcp` — which is how vaults that predate the registry join it with no migration. Registration never fails the command it rides along with.
- `read`, `write`, `lock`, and `unlock` take `--vault` optionally. Without it, the CLI canonicalizes the working directory and uses the **longest registered root that contains it**, so a vault nested inside another is found as itself. A folder in no registered vault is `NOT_INITIALIZED`, with a message saying to pass `--vault` or run `heimdall create`.
- Paths typed at the shell resolve **relative to where the command stands**: the working directory when the vault was found from it, the vault root when `--vault` named it. `..` may climb out of the working directory but never out of the vault (`PATH_OUTSIDE_VAULT`). An absolute path is accepted too — the whole path is optional, not forbidden — and must lie inside the vault. Omitting the path means that same place: `heimdall read` reads the current folder, and `heimdall lock` locks it.
- Output paths are always vault-relative, however the input was spelled.
- Client operations (`create-folder`, `move-path`, `relink`, `delete-path`, `link-graph`) and `mcp` still **require** `--vault`: they are the desktop's and the server's, and both always say which vault they mean.

```text
$ cd ~/Notes/projects
$ heimdall read                        # lists ~/Notes/projects
$ heimdall read my_project.md          # reads projects/my_project.md
$ echo "# Plan" | heimdall write plan.md
$ heimdall lock                        # locks projects/
$ heimdall unlock ../ideas/hello_world.md
```

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

- Tool calls never contain vault paths or vault IDs. Paths in tool calls are vault-relative; the vault root is the default.
- MCP never falls back to the process working directory or the registry; `--vault` is required.
- Any existing folder can be served. There is no initialization step and no structure a vault must have.
- To use multiple vaults, configure multiple server entries (`heimdall-personal`, `heimdall-work`), each with its own `--vault`. Native multi-vault mode is deferred (see §13).
- Client-provided MCP Roots do not expand authority in V1.

## 8. Bounded discovery and reads

No operation reads an entire vault without limits. `read` returns one bounded page of a
folder or one bounded range of a note. `link_graph` (§15) scans the whole vault and is
bounded by node, per-file, and total-byte caps rather than by pagination.

### Listing limits

- Default page size: 50 entries. Maximum page size: 200 entries.
- Listings return metadata only, never file content.
- Do not hash file content during listing. Revisions are returned by reads of a note and by successful writes.
- A folder `read` is non-recursive by default. Recursive listing requires an explicit flag, defaults to a depth of 4, and has a maximum depth of 16.
- Entries sort by relative path, byte-wise, over the whole result set rather than by traversal order — that total order is what makes "resume strictly after this path" well defined.
- A folder `read` returns directories and `.md` files. Vaults may hold other files, but no operation can read one, so listing them would be a dead end. Hidden entries (anything beginning with `.`), filesystem debris such as `.DS_Store`, Heimdall's write-temp sidecars, and the `.lock` sidecars older versions left beside notes are never listed.
- A folder `read` uses stateless continuation: when another page exists, the response includes `next_cursor`, which is simply the last relative path returned. The next call passes it as `cursor` and listing resumes strictly after that path under the same parameters. A malformed cursor is `INVALID_INPUT`. There is no server-side cursor state to invalidate.
- As a safety guard, one folder `read` resolves at most 10,000 filesystem entries; if the guard is hit, return the partial page with `next_cursor`. Only entries beyond the cursor count against the guard: ground a previous page already covered was paid for by that page, and charging for it again would make the far end of a large vault permanently unreachable. Entries a call can reject on name alone — hidden, debris, not Markdown, already behind the cursor — cost nothing.
- Listings are weakly consistent while files change; clients may re-list when they need a fresh snapshot.

### Read limits

- A `read` of a note returns one range of one file. There is no multi-file read: a caller that wants several notes asks for each, and each answer carries its own revision and lock state.
- Default range: 200 lines. Maximum requested range: 1,000 lines.
- Default returned content: 64 KiB. Hard maximum: 256 KiB.
- Split only at UTF-8 and line boundaries.
- Return `complete: false` and `next_line` when more content remains.
- Never silently discard or imply that a partial file is complete.

### Link graph limits

`link_graph` is a client operation (§2, §15): it indexes the whole vault in one call,
and is not an MCP tool. It returns no file content — only paths, titles, sizes,
modification times, lock state, and link endpoints.

Being whole-vault, it is bounded by caps rather than by continuation:

- At most 5,000 nodes. Beyond that the response sets `truncated.node_cap_hit` and counts the Markdown files it did not carry.
- At most 256 KiB scanned per file. A larger file is still a node, with `scanned: false`.
- At most 64 MiB scanned in total. When that budget is spent, the remaining nodes come back with `scanned: false` and `truncated.total_bytes_cap_hit` is true.
- At most 1,000 links extracted from any one file.

A cap is a truncated success, not a failure — the same treatment a folder `read` gives
its scan guard. `scanned: false` is the difference between "this note has no links" and
"its links were not read", and the response never conflates the two.

`relink` (§15) reads the vault under those same caps and adds one of its own: at most
1,000 files written in a single call. A read budget bounds how much of a vault an
operation looks at; a write budget bounds how much of it one rename can change, which is
a different promise and needs saying separately. Past it the response reports what it did
not reach, so the count of links that moved is never larger than the truth.

Edge endpoints are indices into `nodes`, not paths. That is a payload decision rather
than a stylistic one: with string endpoints a 5,000-note vault would nearly fill the
desktop bridge's stdout ceiling, and the failure would arrive as unreadable output rather
than as an honest truncation flag.

Backlinks are the reverse of `edges` and are computed by the client. Returning them as
well would duplicate every edge and let the two drift apart.

Every note read returns a `document` of this shape (§10):

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

## 9. MCP surface

V1 is tools-only for broad client compatibility. The surface is two tools, and the shell has a subcommand of exactly the same name for each — `heimdall read`, `heimdall write` — mirroring the same core operation. The names are the plain verbs: no `read_file`, `read-documents`, or other qualified spelling.

| Tool | Use | Mutates |
|---|---|---|
| `read` | Read a folder (a bounded listing) or a note (a bounded range of its lines). Omit `path` for the vault root. Reports lock state. | No |
| `write` | Create a note, or replace one given the revision from the latest read. Never overwrites silently; refused with `LOCKED` on a locked path. | Yes |

Two tools total. Do not add a general `read_file`, `write_file`, or `execute` tool. Vault creation is shell-only (§7).

There is deliberately no agent-instructions file and no tool to read one. MCP already has a channel for telling a client how to behave — the `instructions` string published at initialization (§12). What varies per vault belongs in the vault's own notes, which `read` reaches like any other.

`lock` and `unlock` are shell subcommands and not tools. A lock decides what an agent may change, so an agent must not be able to change it — not by unlocking, and not by locking either, since a folder lock clears the note-level rules beneath it (§6). This is unconditional rather than a server flag: a flag would make the weaker setup the default. Their request and response types derive no `JsonSchema`, so, like the client operations below, neither can be given a tool without a compile error. An agent that meets a lock reports it — `LOCKED` carries `locked_at` — and the person decides.

The desktop's client surface is separate, and larger (§15). It is reached only through
shell subcommands, and its request and response types deliberately derive no
`JsonSchema` — `rmcp` builds a tool's schemas from those types, so a client operation
cannot be given a tool without a compile error. "Two tools" is therefore a property the
compiler holds, not a promise review has to keep.

## 10. Tool contracts

### `read`

Input (every field optional):

```json
{
  "path": "projects",
  "start_line": null,
  "max_lines": null,
  "max_total_bytes": null,
  "recursive": false,
  "max_depth": null,
  "cursor": null,
  "limit": 50
}
```

- `path` is a folder or a Markdown note, vault-relative; omitted, it is the vault root. A path that does not exist is `NOT_FOUND`; a hidden path (including `.trash/`) or a non-Markdown file is `INVALID_INPUT`; one that resolves outside the vault is `PATH_OUTSIDE_VAULT`.
- `start_line`, `max_lines`, and `max_total_bytes` apply to a note; `recursive`, `max_depth`, `cursor`, and `limit` to a folder. An option for the other kind is `INVALID_INPUT` naming the parameter, rather than being ignored.
- Limits and continuation follow §8.

A folder returns a listing, never content or revisions:

```json
{
  "path": "projects",
  "kind": "directory",
  "locked": false,
  "listing": {
    "entries": [
      { "path": "projects/my_project.md", "kind": "document", "size_bytes": 912,
        "modified_at": "2026-08-16T14:30:00Z", "locked": false }
    ],
    "next_cursor": null,
    "scan_guard_hit": false
  }
}
```

A note returns one bounded range and its revision:

```json
{
  "path": "projects/my_project.md",
  "kind": "document",
  "locked": true,
  "locked_at": "projects",
  "document": { "path": "projects/my_project.md", "content": "# My Project\n", "start_line": 1,
                "end_line": 2, "next_line": null, "complete": true, "size_bytes": 13,
                "revision": "blake3:..." }
}
```

`locked_at` appears only when `locked` is true and names the rule responsible (`""` for the vault root). The response is one object with the payload nested under `listing` or `document`, not a union of two shapes, because an MCP `outputSchema` must be an object.

### `write`

Input:

```json
{
  "path": "projects/my_project.md",
  "content": "# My Project\n\nUpdated.\n",
  "expected_revision": "blake3:..."
}
```

Behavior:

- Writes one complete Markdown note: a whole-file replacement, never a patch or an append.
- **Without `expected_revision`** (absent or `null`), it creates the note and never replaces one: if the note exists, the result is `REVISION_CONFLICT` with `details.current_revision`. The shell spells the explicit form `--create`; it is also the default.
- **With `expected_revision`**, it replaces the note only if that revision is still what is on disk. A mismatch is `REVISION_CONFLICT` with the current revision but not the current content; a missing note is `NOT_FOUND`.
- The note's folder must already exist (`NOT_FOUND`, naming the folder). One mistyped path must not scatter directories through the vault; the desktop has `create_folder` for a new one.
- A locked note, or a new note directly inside a locked folder, is `LOCKED` (§6).
- Hidden paths and non-Markdown paths are `INVALID_INPUT`. Content is limited to 1 MiB (`LIMIT_EXCEEDED`); the shell applies the same bound to stdin.
- Revision comparison, the lock check, and the replacement happen as one step under the vault's cross-process write lock (§14). The write lands in a temporary sibling, is flushed, and is atomically renamed.
- Returns `path`, `new_revision`, `size_bytes`, and `created`.

That an absent revision means "create" rather than "error" is deliberate: it makes the common case — writing a new note — one argument shorter, and it still can never replace anything, which is the property the older "absent is a mistake" rule existed to protect.

### `lock` and `unlock` (shell only)

Not MCP tools (§9); the shell takes `path` positionally (§11), and the core request is:

```json
{ "path": "projects" }
```

- `path` is a folder or a Markdown note, vault-relative; omitted, it is the whole vault. It must exist: `NOT_FOUND` otherwise, and `INVALID_INPUT` for a hidden path or a non-Markdown file.
- Semantics follow §6: a folder operation applies to everything beneath it and clears the rules there.
- Returns `path`, `kind` (`directory` or `document`), `locked` (the new state), and `changed` — false when the path was already in that state and nothing was stored.
- Changes no file in the vault. The rules are written, under the vault's write lock, to the application-data directory (§14).

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

Subcommands:

```text
heimdall read   [PATH] [--vault P] [--start-line N] [--max-lines N] [--max-total-bytes N]
                       [--recursive] [--max-depth N] [--cursor C] [--limit N]
heimdall write  PATH   [--vault P] [--expected-revision R | --create]      # content on stdin
heimdall lock   [PATH] [--vault P]
heimdall unlock [PATH] [--vault P]

heimdall create NAME [--root P]
heimdall create-folder --vault P --path PATH
heimdall move-path     --vault P --from PATH --to PATH
heimdall relink        --vault P --from PATH --to PATH [--dry-run]
heimdall delete-path   --vault P --path PATH [--expected-revision R]
heimdall link-graph    --vault P [--max-depth N]
heimdall mcp           --vault P
```

`read`, `write`, `lock`, and `unlock` take `PATH` positionally and find the vault from the working directory when `--vault` is omitted, resolving `PATH` relative to it (§7). Every other domain subcommand requires `--vault <path>`, matching the rule the MCP server follows: the desktop and the server always say which vault they mean. `heimdall create` instead takes `--root`, which defaults to the working directory, because it is naming a location rather than selecting an existing vault.

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
- `NOT_INITIALIZED` — the shell was asked to find a vault from a working directory that is inside no registered vault (§7). Nothing else raises it: any folder named with `--vault` is a vault.
- `REVISION_CONFLICT`
- `LOCKED` — the path, or the folder it would be created in, moved into, or moved out of, is locked (§6). `details.locked_at` names the rule.
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

> Heimdall manages one Markdown vault with two tools. Start with `read` and no path: it lists the vault root. `read` on a folder lists it; `read` on a note returns a bounded range of its lines and the revision a write needs. Request only the ranges you need and follow `next_line` and `next_cursor`. `write` creates a note, or replaces one when you pass the `expected_revision` from your latest read; it never overwrites anything silently. Every read says whether a path is `locked`. A locked note or folder is read-only, and only the user can change that: locks are set outside this server. If a write is refused with LOCKED, tell the user which lock is responsible (`locked_at`) instead of working around it.

What is fixed for every vault belongs in that string; what varies per vault belongs in
the vault's own notes. Every tool also receives a strong description explaining when to
use it, what it cannot access, its limits, and whether it mutates content.

## 13. Roadmap (deferred by design)

Deferred features must preserve the same vault restrictions, limits, revisions, and continuation behavior.

- **MCP resources.** Read-oriented content is semantically suitable for resources later (`heimdall://notes/<relative-path>`). Adding resources does not remove `read`; mutation remains tools.
- **Native multi-vault mode.** Friendly vault IDs on every call plus a `list_vaults` tool. Until then, one server process per vault (§7).
- **MCP Roots** as an explicit, reviewed configuration source.
- **Non-Markdown file operations**, only with explicit MIME, size, and security rules.

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
- Application data, not a cache directory: a cache is something the system may purge, and a purge that unlinks a held lock file is the same divergence that makes lock files unsafe to delete — and a purge of the lock rules would silently unlock every note. `HEIMDALL_DATA_DIR` overrides the location for a deployment with no writable home, and for tests.
- The vault root is canonicalized when the vault is opened, because the lock key is what two processes must agree on and a path as typed is not that — `~/vault`, `./vault`, and a symlink to it are three spellings of one directory that would otherwise take three separate locks and exclude nothing.
- One lock per vault is coarser than one per file and therefore strictly stronger, so it cannot introduce a race. A holder only hashes some bytes and renames a temp file, so serialising a vault's writes costs nothing at the rate they arrive. It does mean a locking operation must never be called from inside another one: with a single lock that is a self-deadlock rather than merely redundant.
- Write temps are the one thing Heimdall does put in a vault, and unavoidably: an atomic write lands in a temporary sibling and is renamed into place, and a rename cannot cross filesystems. It is removed on every path including failure, so one survives only a kill or a power loss.
- Bound the wait for a contended lock (10 seconds) rather than blocking indefinitely. Every holder does short, bounded work, so exceeding that means another process is stuck, and reporting `IO_ERROR` beats inheriting its hang — a server cannot promise a bounded operation duration on top of an unbounded wait.
- A rename never replaces an existing destination. `cap-std` exposes no `RENAME_NOREPLACE`, so the check and the rename both happen under the vault's write lock, and a collision is `ALREADY_EXISTS`. This serialises Heimdall processes against each other, not against another editor writing into the same vault — the same weak consistency §8 already accepts for listings.
- Deletion moves content into `.trash/`; nothing is ever unlinked. Collisions there take numeric suffixes (`note_01.md`, `note_02.md`, …).
- **Everything Heimdall keeps lives in one per-user application-data directory**, `HEIMDALL_DATA_DIR` if set and otherwise the platform's local data directory plus `heimdall/`:

  ```text
  <data>/
  ├── vaults.json            # the registry: canonical vault roots (§7)
  ├── locks/<key>.lock       # one write lock per vault
  ├── locks/registry.lock    # serialises registry updates
  └── vaults/<key>.json      # one set of lock rules per vault (§6)
  ```

  `<key>` is the BLAKE3 hash of the vault's canonical path, the same key for the write lock and the lock rules. Both JSON files are written through a temporary sibling and a rename, so a reader sees the old state or the new one, never half of either.
- **Lock rules are read and changed under the vault's write lock.** `lock` and `unlock` rewrite the rules inside it; every write checks them inside the same lock body in which it compares revisions and renames. So an `unlock` cannot land between a write's check and its rename, and a `lock` returning success means no write that began before it is still to land. `read` reads the rules without the lock — its `locked` flag is weakly consistent, like a listing.
- The registry is updated under its own short lock so two processes registering at once cannot each write a list missing the other's vault.
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

1. **Files** — the whole vault, from a recursive `read` of the root. New note, new folder, sort order, collapse-all, and a lock for the whole vault sit in a toolbar above the tree. A right-click offers rename, delete, and lock or unlock. A locked note or folder carries a small lock glyph in the tree, drawn in the muted grey rather than the accent; it offers unlock and nothing that would change it, and it cannot be dragged or dropped onto, since every one of those operations would be refused (§6). A move that would break inbound links — a rename, a folder rename, or a drag — stops and asks first, **naming** the notes it would rewrite rather than counting them, since those are the files being agreed to. Three answers: update the links, rename only, or cancel; nothing at all is written until one is chosen, so cancelling leaves the vault as it was, and "rename only" is a real position rather than something the dialog talks you out of. A move that breaks no links does not ask, because a rename that touches one file is not worth a dialog. Afterwards nothing is said when every link was carried — the user agreed to it and is looking at the result — and a dialog reports anything left behind, which is the one outcome they would otherwise meet later as a dead link. A folder is asked about its contents, since nothing links to a directory. A delete still warns rather than offering: deletion has no new name to point links at.

A report of that kind must not be a banner. The workspace clears its banner whenever the vault reloads, and autosave triggers a reload a second and a half after any edit — so a banner raised by a write is wiped by a refresh nobody asked for, usually before it has been read. The banner's own vault-level failure is the one thing a successful reload may clear, because it is the one thing a successful reload disproves.
2. **Note** — a breadcrumb with back/forward history, a source/preview toggle, and an overflow menu. The heading is the note's own `# ` line, and it is the filename: one fact, so editing the heading renames the file and renaming the file rewrites the heading. It is shown in both modes and typed into in one — source, with its `#` dimmed like every other marker, drawn above the editor rather than inside it so that it cannot be scrolled away from and the same line is not on screen twice. In preview it is a rendered heading and inert, because preview is for reading and a rendered heading that quietly accepts typing is one nobody can tell from the rest of the note. It cannot be removed: a note emptied of its name gets the name back, and a note that never had a heading is given one from its filename the first time it is written. Preview renders the note, shows its frontmatter as a properties table with clickable wikilinks, renders fenced `mermaid` diagrams, and lists linked mentions — the notes that link to this one, each a name to click beside its path and nothing else. The note's own outgoing links are not repeated under it: they are in its text a few lines above, and a second copy is one more list to read past. The properties block is found wherever its author put it, which for a note that opens with its name is under the heading rather than on the first line; because `---` is a horizontal rule as well as a fence, a block counts only when it is closed, sits at the top of the note or under a blank line, and encloses a non-empty mapping — and only the first such block is the note's properties, since a note has one set of them. Every other `---` is the rule it looks like, and no rule is drawn that the note does not contain. Source is a Markdown editor with the frontmatter block and the remaining heading markers dimmed.
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

1. **Vault** — create a new templated vault (`heimdall create`) or select an existing folder of notes, which is registered as a vault as it stands: nothing is written into it.
2. **Server** — make the MCP server available: show the exact `heimdall mcp --vault ...` command, generate each known client's entry in its own format (JSON for Claude Desktop, TOML for ChatGPT), and (with explicit user consent) write it into a known client's configuration file. The known clients are Claude Desktop (`~/Library/Application Support/Claude/claude_desktop_config.json`, an `mcpServers` entry) and ChatGPT (`~/.codex/config.toml`, shared with the Codex CLI, an `[mcp_servers.<name>]` table); each file is written in its own format, and a TOML file keeps its comments and layout. Writing merges into the existing configuration rather than replacing it, backs the previous file up first, and saves through a temporary sibling so an interrupted write cannot leave a client with half a file. A second vault gets its own entry name instead of taking over the first one's. A development build refuses to write at all, and the screen says so before the click: its sidecar is a build artifact that a rebuild or `cargo clean` removes, and a client whose configured command has gone reports a timeout rather than a missing file — so the entry would fail silently and much later. For the same reason the screen reports a registration that is already stale: an entry for this vault whose absolute command is no longer on disk. Provide a one-click health check that launches the server, performs an MCP handshake, and reports the result.
3. **Appearance** — System, Light, or Dark, and an accent colour for each theme. System is the default and is applied by the stylesheet's media query, so the first paint is correct without waiting for JavaScript. The accent is offered as a colour well beside the hex it resolves to, with a control that clears the choice rather than writing the default back; light mode has no hue to set.
4. **Diagnostics** — active CLI path and versions, protocol/schema compatibility, configured vault path and whether it can be read, limits, and recent actionable errors.

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

### Locks in the desktop

Locks bind the desktop exactly as they bind an agent (§6): the editor is one more writer.

- A locked note opens read-only: the editor accepts no input, the heading cannot be renamed, and the properties table offers no controls. The note's header carries a lock toggle, and the tree's context menu offers the same.
- Locking or unlocking the open note changes whether it is editable **in place**. The editor's read-only state is reconfigured rather than the editor rebuilt, so the cursor, the scroll position, and the undo history all survive.
- A write the CLI refuses with `LOCKED` — a lock set from the shell or by an agent while the note was open — is reported in a dialog naming the lock responsible, never in the banner (a banner raised by a write is wiped by the reload autosave triggers), and autosave does not retry those bytes.
- `relink` skips locked notes (§6) and reports them; the rename's follow-up dialog names them as links left behind.

### Client operations

Editing needs structural writes that the MCP surface does not have and must not grow (§9).
These are shell subcommands the desktop calls, exactly as `heimdall create` is (§7). Saving
a note is not among them: the editor saves through `write`, the same operation an agent uses,
so a lock or a stale revision refuses the editor exactly as it refuses anyone else.

| Command | Use |
|---|---|
| `create_folder` | Make a folder, and any missing parents. Refused inside a locked folder. |
| `move_path` | Rename or move a note or folder. Refused for anything locked, and into or out of a locked folder. |
| `relink` | Retarget the links that pointed at a path `move_path` has just changed. Never writes a locked note. |
| `delete_path` | Move a note or folder into the vault's `.trash/`. Nothing is ever unlinked. Refused for anything locked. |
| `link_graph` | Index the whole vault's links in one call (§8). Each node reports whether it is locked. |

Rules:

- None of these is an MCP tool, and none of their types derives `JsonSchema`. The tool surface stays at exactly two (§9), and a compile error is what enforces it.
- A stale revision returns `REVISION_CONFLICT` with the current revision in `details`, and the client resolves it by asking the user — never by merging and never by overwriting.
- Renaming a note does not rewrite `[[wikilinks]]` **inside `move_path`**. A move is one rename, and folding an unbounded multi-file write into it would leave a rename that half succeeded with no way to say so. `relink` is that work, as its own operation with its own report, and the client calls it next.
- `relink` is bounded and checked, which is what the older rule was protecting. **Bounded:** only files holding a link to the moved path are written; the scan is the whole vault under the graph's caps (§8) plus a substring prefilter, and the writes are capped again. **Checked:** the whole read-modify-write runs inside one write lock, which is strictly stronger than an `expected_revision` a caller could pass — a revision check closes the gap between a client's read and its write, and here there is no gap. That is also why it uses the vault primitives directly rather than `write`: a locking operation inside a lock body is a self-deadlock (§14).
- A rewrite is **proposed and then verified**. The replacement is written in the shape the link was written in — a bare name stays bare, a vault path stays a path, a note-relative link is re-expressed from the linking note, an extension and a percent encoding are preserved — and is then resolved again through the post-move index. It is spliced in only if it lands on the moved note; if it does not, because the new basename is now ambiguous, the vault-relative path is tried instead. A link that no proposal satisfies is reported and left exactly as written. Retargeting a link at the wrong note is worse than leaving one broken: the first is invisible.
- Only the target substring is replaced, so an alias, a `#heading` or `#^block` anchor, an `![[` embed marker, and a Markdown link's text and title come out byte-identical. Frontmatter is never reassembled, so every other key in it survives when a link inside it moves.
- A locked note that links at what moved is not rewritten; it is listed in the response's `locked`, so the client can name the links a lock kept pointing at the old path.
- Both ends of a link can be what moved: a note that changed folders takes its own note-relative links with it, and those are re-expressed from its new home.
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
- **One accent per theme**, both the user's, set in Settings → Appearance. Not one accent shared between them: no single colour works on both grounds, since white is the right accent on black and invisible on white and black is the reverse, so a shared accent makes every choice a compromise and the obvious ones unusable in one theme. Each is used as picked, neither lightened nor darkened, and the Appearance copy says to choose one that reads against its own background. The accent is spent on links, the row of the note you have open in the file tree, the active graph node, and mermaid diagrams, and nothing else — not on lock glyphs, which are drawn in the muted grey. The open note is *named* in the accent rather than sat on a block of grey: a selected-row highlight says "list box", and nothing else in this application talks that way.
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
- `read`, `write`, `lock`, and `unlock` receive their `path` positionally, after `--`, so a note whose name begins with a dash is still only a path. The desktop always passes `--vault`, so its paths are vault-relative whatever the sidecar's working directory is (§7).
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

Heimdall is source-available under the Sustainable Use License 1.0 (`LICENSE`), not an open-source license. Every artifact carries `LICENSE`, and the desktop bundles it at `Heimdall.app/Contents/Resources/LICENSE`. The Cargo manifests name it with `license-file`, because it has no SPDX identifier.

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

### Desktop distribution (macOS)

The macOS desktop ships as `Heimdall_<version>_aarch64.dmg`, for Apple Silicon only (macOS 11 or later). An Intel or universal build is not produced.

- `Heimdall.app` and its bundled `heimdall` sidecar are signed with a Developer ID Application identity, with the hardened runtime and a secure timestamp, and need no entitlements. The app is notarized, and its ticket is stapled.
- The DMG is signed, notarized, and stapled too, so Gatekeeper accepts it offline.
- `npm run release:mac` (`apps/desktop/scripts/release-macos.mjs`) produces the release. It takes the identity and the App Store Connect API key from `APPLE_SIGNING_IDENTITY`, `APPLE_API_KEY`, `APPLE_API_ISSUER`, and `APPLE_API_KEY_PATH`, and none of them is stored in the repository. It refuses to finish unless every signature checks out, Gatekeeper reports the app as notarized, both tickets validate, and the bundled CLI reports the app's version.
- The app is meant to run from `/Applications`: the client configuration it writes names the sidecar's absolute path inside the bundle.
- There is no separate CLI artifact on macOS. A user who wants `heimdall` on their `PATH` links the bundled sidecar, which keeps it signed and version-matched across upgrades. Linking is optional, so installation itself still needs no administrator privileges.
- Uninstalling removes the app. The per-user application-data directory (§14) is left for the user to delete, because deleting it unlocks every note. Vaults are never touched.

## 17. Testing strategy

### Core tests

- `heimdall create` writes the template's example notes into an empty or missing target, copies no filesystem debris, and registers the vault
- `heimdall create` on a folder that already has content writes nothing into it (hidden files included) and registers it
- `read` of a folder: pagination via `cursor`/`next_cursor` continuation, deterministic ordering, and the entry-scan guard; hidden folders and `.trash/` never listed or readable
- `read` of a note: line continuation, byte caps, and options for the other kind refused rather than ignored
- UTF-8 boundaries and invalid UTF-8 errors
- Traversal and symlink escape rejection
- Atomic writes and interrupted-write recovery
- `write`: revision match and conflict; an absent or null revision creates and never replaces; a missing folder is named rather than created; concurrent creates of one path leave exactly one winner
- Locks: a folder lock reaches every depth; a note can be unlocked inside a locked folder until the folder is locked again; unlocking a folder unlocks notes locked on their own; a root lock locks everything; every refused operation — write, create, create-folder, move from, move into, move out of, delete, delete of a folder holding a locked note — is `LOCKED` with the right `locked_at`; relink skips locked notes and reports them; locking twice is not a change; only existing folders and notes can be locked; nothing is written into the vault
- The registry: a subfolder finds its vault and its place in it; the nearest root wins for nested vaults; a sibling with a longer name is not inside; a symlinked working directory finds the real vault; a folder in no vault is `NOT_INITIALIZED`; registering twice records once; shell paths join the working directory, climb with `..` but never out of the vault, and accept an absolute path inside it
- Cross-process write locking, and a lock racing writers never half applied

### CLI/MCP contract tests

- Shell and MCP adapters produce equivalent domain outcomes for `read` and `write`, successes and failures alike (`NOT_FOUND`, `LIMIT_EXCEEDED`, `INVALID_INPUT`, `REVISION_CONFLICT`, `LOCKED`).
- A write or a lock through one adapter is visible through the other.
- Shell output retains its envelope.
- MCP output uses typed `structuredContent` without the shell envelope, and every result has the fields its published output schema requires.
- Expected domain failures and protocol failures are distinguished; arguments that violate a tool's schema still carry `INVALID_INPUT`.
- stdout contains protocol/JSON only; stderr cannot corrupt it.
- The configured vault boundary is enforced; no tool call can supply a vault path, and an absolute path is read as a path inside the served vault.
- Run from a folder inside a vault, `read`, `write`, `lock`, and `unlock` need no `--vault`, take the current folder by default, and resolve paths relative to it; from a folder in no vault they report `NOT_INITIALIZED` with guidance.
- The MCP server advertises exactly `read` and `write` — not `lock`, `unlock`, or any client operation — checked both in-process and over a real stdio connection; calling `lock` or `unlock` over MCP is a protocol error and leaves the lock rules untouched; a lock set at the shell is reported by an MCP `read` and refuses an MCP `write`.

### Client operation tests

- Deletion lands in `.trash/`, keeps the original layout, takes a numeric suffix on collision, and disappears from listings.
- A move refuses to leave Markdown behind, refuses a folder into itself, and never replaces an existing destination.
- Link resolution follows the conventional wikilink rules, including the shortest-path tie-break, and is byte-for-byte deterministic across runs.
- Every link span slices back to the target exactly as written, including past a byte-order mark, across a masked inline-code span, and for a percent-encoded Markdown destination. A span off by one byte corrupts a note, so this is checked directly rather than through its callers.
- A rename carries a bare name, a vault path, and a note-relative link, each in its own written form; an alias, an anchor, an embed marker, a Markdown title and angle brackets all survive byte-identical.
- A rename into a now-ambiguous basename escalates to the vault-relative path rather than retargeting the link at the wrong note.
- A link in fenced or inline code is never rewritten; one in real frontmatter is. A note with no link to the moved path is not written at all, and `.trash/` is left alone.
- A folder rename carries every link into it, and a moved note's own relative links are re-expressed from its new home.
- A dry run reports the revision the real write produces and writes nothing; a second run over a settled vault changes nothing.
- Links inside fenced code, inline code, and unterminated frontmatter produce no edges; links inside real frontmatter do.
- The graph carries no file content, reports each node's lock state, and truncates at each cap while reporting what it left out.

### Desktop tests

- The bundled sidecar is invoked (never a `PATH` binary), without a shell, with Markdown over stdin.
- Setup creates a templated vault and registers an existing folder without writing into it.
- The Server screen generates a valid client config and the health check completes an MCP handshake.
- Structured failures remain actionable and do not crash the UI.
- Theme, border, and typography rules render correctly in dark and light modes, and each theme's accent reaches the page only through a custom property. One theme's accent is never the other's.
- No colour literal appears anywhere in the TypeScript source; the editor theme and the graph renderer read the stylesheet.
- A whole note is assembled from bounded reads, restarts when the file changes mid-read, and compares bytes rather than characters when checking the result.
- A locked note opens read-only; locking or unlocking the open note flips its editability without rebuilding the editor; the tree marks locked rows, offers lock and unlock, and neither drags nor drops onto a locked path; a `LOCKED` refusal is reported in a dialog.
- Frontmatter round-trips without reformatting the keys the user did not touch.
- Markdown renders as elements: raw HTML in a note is shown as text, never mounted.
- Settings opens from the application menu event and closes from both the X and Escape.

### Release smoke tests

For every supported operating system:

1. Install only the CLI and exercise all headless operations, including `heimdall create`, and `read`/`write`/`lock` from inside the vault without `--vault`.
2. Install the desktop app on a clean user account; create a vault, run the health check, and install a client config.
3. Open an existing folder of notes and verify that its files, hidden ones included, remain unchanged after it is registered and after notes in it are locked.
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

Delivered for macOS on Apple Silicon (§16). Still open: an Intel or universal macOS build, a standalone CLI archive, and other platforms.

- Produce signed CLI artifacts and desktop installers.
- Bundle the correct CLI sidecar for each target triple.
- Verify clean installation, upgrades, and uninstallation.

### Phase 5 — Desktop editor

Delivered ahead of Phase 4, which does not block it.

- Add the client write operations and `.trash/`.
- Add the link graph and its bounds.
- Build the three-pane workspace, the Settings modal, and the application menu.
- Leave the MCP surface unchanged.

### Phase 6 — Read, write, and locks

Delivered ahead of Phase 4, which does not block it.

- Remove the protected `aios/` tree, memories, and entries; every visible note is ordinary content.
- Collapse the agent-facing surface to `read` and `write`, on the shell and over MCP.
- Add locks at vault, folder, and note level, with `lock` and `unlock` on the shell and in the desktop, and lock state reported over MCP.
- Add the vault registry so the shell works from inside a vault with paths relative to the working directory.

## 19. Definition of done

- The installed executable and MCP command are named `heimdall`.
- The CLI works without the desktop application, and from inside a vault needs no `--vault`.
- `heimdall create` produces the templated vault and safely registers an existing folder of notes.
- The desktop uses its bundled CLI for every domain operation and can install a working MCP client configuration.
- No operation performs an unbounded vault read.
- The MCP surface is exactly `read` and `write`; neither the lock controls nor any client operation is reachable through it.
- MCP results use typed structured content rather than the shell envelope.
- The MCP surface has no vault-path or vault-creation capability.
- Stale writes fail with `REVISION_CONFLICT`, and a write without a revision never replaces a note.
- A locked note cannot be written, moved, or deleted by any surface, the desktop editor included, until it is unlocked.
- No write can escape the vault or silently replace unrelated content.
- No deletion unlinks user data.
- Nothing of Heimdall's is left in a vault.
- Vault data survives CLI/Desktop upgrades and uninstallation.

## 20. Decisions retained from the original requirements

- `write` replaces a complete note; it does not patch or append.
- A vault may contain non-Markdown files, but no operation reads or writes them.
- Deleted content goes to `.trash/` and is never unlinked.

## 21. Verified implementation references

Verified on 2026-08-16:

- [Tauri 2: Embedding External Binaries](https://v2.tauri.app/develop/sidecar/) — `externalBin`, target-triple naming, sidecar execution, and scoped permissions.
- [Tauri 2: Distribution](https://v2.tauri.app/distribute/) — platform packages, signing, and notarization.
- [Official MCP Rust SDK](https://github.com/modelcontextprotocol/rust-sdk) — the `rmcp` SDK and standard local stdio transport.
- [MCP Rust SDK roadmap](https://github.com/modelcontextprotocol/rust-sdk/blob/main/ROADMAP.md) — protocol and conformance status.

At the verification date, the newest protocol support in the official Rust SDK was still progressing through Tier 1/conformance work. Pin a known-good `rmcp` release and protocol baseline instead of automatically tracking the newest revision.
