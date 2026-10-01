# Heimdall — Agent Instructions

Heimdall is an intent-aware MCP layer for Markdown vaults: a headless Rust CLI (`heimdall`) that provides shell commands and a stdio MCP server, plus a Tauri desktop app that both configures the server and edits the vault — a file tree, a Markdown editor with preview, and an interactive link graph. Agents get two verbs, `read` and `write`; people get locks (vault, folder, note) that make any of it read-only. The full product behavior is specified in **`docs/SPEC.md`** — that document is the source of truth. Read the relevant section of the spec before changing any contract, limit, or tool schema.

## Repository layout

- `crates/heimdall-core/` — all domain rules, validation, limits, revisions, and filesystem operations.
- `crates/heimdall-cli/` — shell command surface (`shell.rs`) and MCP adapter (`mcp.rs`); both are thin translations into core operations.
- `apps/desktop/` — Tauri 2 + React + TypeScript + Vite. `src-tauri/` holds the Rust bridge to the bundled CLI sidecar and the application menu. `src/api/` is the only route to the domain; `src/features/` holds the three panes, Settings, and the quick switcher; pure logic (tree shape, link resolution, fuzzy ranking, graph maths, frontmatter) lives beside its component and is unit-tested without a DOM.
- `docs/SPEC.md` — the specification. `docs/vault-template/` — the canonical default vault contents (example notes only) embedded verbatim into the binary.
- Integration tests live in each crate's own `tests/` directory; a virtual workspace root has no test target.

**Status: Phases 1–3, 5, and 6 (SPEC §18) are implemented** — core, the direct shell CLI, the MCP stdio server (`heimdall mcp --vault <path>`), the Tauri desktop app, the desktop editor with its client operations and link graph, and the read/write/lock surface with the vault registry. The protected `aios/` tree, memories, and entries are gone. Phase 4 is delivered for macOS on Apple Silicon: a Developer ID–signed, notarized, and stapled DMG (SPEC §16). An Intel or universal build, a standalone CLI archive, and other platforms are not done.

`apps/desktop/src-tauri` is deliberately **not** a workspace member: `tauri::generate_context!` embeds the built frontend, so including it would break `cargo build --workspace` until Vite has run, and it would drag a WebView dependency into the CLI artifact.

## Hard boundaries (never violate)

- `heimdall-core` owns every filesystem operation. The CLI, MCP adapter, and desktop bridge never touch the filesystem directly.
- Never add a general `read_file`, `write_file`, or `execute` tool to the MCP surface.
- The MCP surface is exactly two tools — `read` and `write` — and the shell has a subcommand of the same plain name for each. Never spell them `read_file`, `read-documents`, or anything qualified, and never add a third.
- `lock` and `unlock` are shell subcommands and **never** MCP tools, in either direction: an agent that can unlock gets past every lock, and one that can lock a folder erases the user's note-level unlocks beneath it. Only people set locks — at the shell or in the desktop. `LockRequest`/`LockResponse` derive no `JsonSchema` for the same reason the client operations don't; never add it back, and never offer an agent a way around a lock in instructions or tool descriptions — a `LOCKED` refusal is reported to the user, with `locked_at`.
- Client operations (`create`, `create-folder`, `move-path`, `relink`, `delete-path`, `link-graph`) are shell-only. Never derive `JsonSchema` on their request or response types, and never give one a tool: the missing derive is what makes "two tools" a compile error rather than a convention. The desktop editor saves through `write`, like everyone else.
- **Locks bind every writer**, the desktop editor included. A locked path cannot be written, moved, or deleted; nothing can be created in, moved into, or moved out of a locked folder; `relink` never rewrites a locked note and reports it instead. Every lock check runs inside the same `with_write_lock` body as the write it guards — never before it — so an `unlock` cannot slip between the check and the rename. The refusal is `LOCKED` with `details.locked_at`.
- Lock semantics live once, in `notelocks.rs`: nearest rule wins; a folder lock or unlock clears every rule beneath it; redundant rules are not stored. Do not re-derive them in an adapter or in TypeScript — the UI reads `locked` from `read`, listings, and graph nodes.
- There is no protected tree. Every visible note is reachable by `read` and `write`; hidden paths (including `.trash/`) are never listed, read, or written.
- Deletion moves content into the vault's `.trash/`; nothing is ever unlinked. A rename never replaces an existing destination.
- `move_path` does not rewrite `[[wikilinks]]`; `relink` does, as a separate operation the client calls next. Keep it that way. A move is one rename, and folding a multi-file write into it leaves a rename that half succeeded with no way to report it. What made the old rule right was *unbounded* and *unchecked*, and `relink` is neither: it writes only the files the index says link at the moved path, and its whole read-modify-write runs inside one write lock — stronger than an `expected_revision`, which exists to close a gap between a client's read and its write that this does not have.
- The desktop asks before it relinks, naming the notes it would rewrite, and offers "rename only" as a real answer. Nothing is written until the dialog is answered. Report a shortfall in a dialog, never a banner: `reload` clears the banner on success and autosave triggers a reload 1.5s after any edit, so a banner raised by a write is wiped before it is read. Only the vault-level failure `reload` itself raised may be cleared that way.
- A `relink` replacement is proposed in the shape the link was written in and then **resolved again** before it is spliced in. Never skip that step, and never reach for string substitution: a bare `[[roadmap]]` renamed to `plan` has to become `[[projects/plan]]` when another `plan.md` would now shadow it. A link no proposal satisfies is reported and left alone — retargeting a link at the wrong note is worse than leaving one broken, because the first is invisible.
- Link resolution lives once, in `commands/resolve.rs`, and both `link_graph` and `relink` use it. Two copies would disagree the first time either was touched, and a rewriter that disagrees with the graph retargets links the reader can see going somewhere else.
- MCP tool calls never accept vault paths. The vault is fixed by `--vault` server configuration (one vault per server process).
- Vault creation is shell-only (`heimdall create`); it is not an MCP tool. On a folder that already has content it writes nothing, and only registers the vault.
- Only the shell discovers a vault: `read`/`write`/`lock`/`unlock` without `--vault` walk up from the working directory to the longest registered root and resolve paths relative to the working directory (`registry.rs`). MCP and the client operations always take `--vault`; with `--vault`, paths are vault-relative. MCP paths are always vault-relative and never absolute filesystem paths.
- Never overwrite user data silently: atomic temp-file-and-rename writes, `expected_revision` checks on replace-style writes, a `write` without a revision only ever creates, numeric suffixes on filename collisions, and deletion to `.trash/` rather than unlinking.
- Path safety uses directory capabilities (`cap-std`), never string-prefix checks. Reject traversal, absolute child paths, and escaping symlinks.
- MCP stdout carries protocol JSON only; all logging goes to stderr.
- Every read and listing is bounded — see SPEC §8 for the limits before touching them.

## Conventions

- Heimdall is source-available under the Sustainable Use License 1.0 (`LICENSE`), owned by Sam Larsen. It is not open source. Never add an OSI license identifier (`MIT`, `Apache-2.0`, …) or license header to our own files or manifests; they use `license-file`. Outside contributions require the CLA (`CLA.md`, enforced by `.github/workflows/cla.yml`). Third-party dependencies keep their own licenses.
- MCP tool names are snake_case; shell subcommands are kebab-case; the two adapters must produce equivalent domain outcomes (contract-tested).
- Shell output uses the versioned `{ok, data|error, meta}` JSON envelope; MCP returns typed `structuredContent` and never the shell envelope.
- Domain errors use the fixed code set in SPEC §11. Add codes deliberately; do not invent ad-hoc ones.
- Revisions are `blake3:<lowercase-hex>` of exact file bytes.
- Timestamps are UTC.
- Use `fs4` (not `fs2`) for the cross-process write lock. There is exactly one per vault and it lives **outside** the vault — in the per-user application-data directory (`appdata.rs`, `HEIMDALL_DATA_DIR` overrides it), named by a hash of the vault's canonical path (which is why `Vault::open` canonicalizes). The lock rules (`vaults/<hash>.json`) and the registry (`vaults.json`) live beside it. Nothing of Heimdall's belongs in a user's vault: a lock file beside a note can never be safely deleted, so that design left one behind for every note ever written. Never call a locking operation from inside a lock body; with one lock per vault that deadlocks — so code inside a lock body reads and writes lock rules with `LockRules::load`/`save` directly, never through `lock()`/`unlock()`/`write()`.
- Tests never touch the real application-data directory: core integration tests use `Vault::open_with_data_dir`, and anything that forks the binary sets `HEIMDALL_DATA_DIR`.
- `heimdall-core` is synchronous. `tokio` belongs to `heimdall-cli`, where `rmcp` needs it; MCP tools run core calls on a blocking worker.
- Tool schemas derive from the core request/response types (`JsonSchema`), so the two adapters cannot drift — and a type without `JsonSchema` cannot be given a tool at all, which is the mechanism keeping the client-only surface off MCP. Tool signatures must spell out `Result<Json<T>, CallToolResult>` — the `#[tool]` macro reads the literal return type to derive `outputSchema`, and a type alias hides it.
- Do not import `heimdall_core::Result` in `mcp.rs`: the rmcp macros expand bare `Result` and would capture it.
- Client-operation subcommands and `mcp` require `--vault`; `read`, `write`, `lock`, and `unlock` take it optionally and take their path positionally. Exit 0 on success, 1 on a domain error (envelope still printed), 2 on a usage error.
- Pin `rmcp` and the MCP protocol baseline; upgrade only after contract tests pass.
- Desktop UI: monochrome (black/white per system theme, with an override in Settings), 1px squared borders everywhere including inputs, Helvetica stack (`"Helvetica Neue", Helvetica, Arial, sans-serif`). Lock glyphs are muted grey, never the accent. A lock toggled on the open note reconfigures the editor's read-only compartment in place; it never rebuilds the editor. One accent **per theme**, spent on links, the active graph node, and mermaid, and reached everywhere through `var(--accent)` / `var(--link)`. Never share one between the themes: white is the right accent on black and invisible on white, and black is the reverse. Settings writes each as an inline custom property on `<html>` — `--accent-light`, `--accent-dark` — neither of which the stylesheet declares: each theme resolves `--accent` from its own with a declared `--accent-*-base` on `:root` as the `var()` fallback, so a choice is clearable, the defaults are opposite ends of the palette, and no theme logic reaches TypeScript. The window draws no title bar of its own (`titleBarStyle: Overlay`), so the workspace runs to the top of the window and the toolbars in that band leave the traffic lights their room through `--traffic-lights`. `src/design.test.ts` enforces all of this by reading `styles.css`, and `src/colour.test.ts` forbids a colour literal anywhere in TypeScript, so the CodeMirror theme and the canvas renderer must read the stylesheet.
- Markdown is rendered by walking `marked`'s tokens into React elements, never by building an HTML string. Agents write into this vault, so markup in a note is ordinary input.
- The desktop always invokes its bundled sidecar CLI — no `PATH` discovery, no shells, Markdown via stdin. React reaches it only through `invoke_cli`, whose subcommand *and* argument keys are allowlisted in `cli_bridge.rs`; the frontend has no shell capability at all. The desktop always passes `--vault`, and the bridge passes `path` for `read`, `write`, `lock`, and `unlock` positionally after `--`.

## Commands

```bash
cargo build --workspace          # build core + CLI
cargo test --workspace           # core, contract, and integration tests
cargo run -p heimdall-cli -- ...  # run the CLI locally

cd apps/desktop
npm install
npm run sidecar                  # build the CLI and stage it as an externalBin
npm test                         # frontend tests (design rules, panes, pure logic)
npm run tauri:dev                # desktop dev
npm run tauri:build              # .app and .dmg (unsigned without APPLE_* set)
npm run release:mac              # signed + notarized DMG; needs the APPLE_* variables (README → Releasing)
(cd src-tauri && cargo test)     # bridge tests; needs `npm run sidecar` first
```

## When making changes

1. Change behavior in `heimdall-core` first; adapters follow.
2. Update `docs/SPEC.md` in the same change when a contract, limit, or schema moves.
3. Add or update the matching tests (SPEC §17 lists the required coverage areas).
4. Keep shell and MCP adapters contract-equivalent.
