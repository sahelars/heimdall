# Heimdall

Markdown vaults your AI can read and write, and you can lock.

Heimdall gives AI clients two verbs over your notes, `read` and `write`, instead
of unrestricted filesystem access. You can **lock** any note, any folder, or the
whole vault, and nothing can change it until you unlock it: not an agent, not a
script, not the editor.

A vault is a folder of Markdown files and nothing else, so any Markdown editor
opens the same files. Heimdall keeps nothing of its own inside it.

Heimdall comes in two parts:

- **Heimdall.app** is a desktop editor with a file tree, a Markdown editor with
  preview, and a link graph. It is also where you connect AI clients.
- **`heimdall`** is the command line tool and MCP server. It ships inside the
  app, and you can also build it on its own.

## Install

Requires macOS 11 or later on Apple Silicon.

1. Download `Heimdall_<version>_aarch64.dmg` from the
   [Releases](https://github.com/sahelars/heimdall/releases) page.
2. Open it and drag **Heimdall** into **Applications**.
3. Launch Heimdall from Applications. The app is signed with a Developer ID and
   notarized by Apple, so macOS opens it after the usual "downloaded from the
   internet" confirmation.

Run Heimdall from `/Applications`, not from the mounted disk image. When you
connect an AI client, the app writes the absolute path of its bundled
`heimdall` into the client's configuration. A path inside `/Volumes/…`
disappears when the disk image is ejected.

### Put `heimdall` on your PATH (optional)

The app bundles a signed command line tool that always matches the app's
version. To use it from a terminal, link it onto your `PATH`:

```bash
sudo ln -sf /Applications/Heimdall.app/Contents/MacOS/heimdall /usr/local/bin/heimdall
heimdall --version
```

If you'd rather not use `sudo`, link it into a directory you own that is already
on your `PATH`, such as `~/.local/bin`. The link follows the app, so updating
Heimdall updates the CLI too.

## Getting started

Open **Heimdall → Settings…** (`⌘,`) and go to **Vault**. From there you can
create a new vault, which starts with a few example notes, or open a folder of
Markdown you already have. Opening an existing folder writes nothing into it.

To do the same from a terminal:

```bash
heimdall create my-vault --root ~/Documents
```

## Connecting an AI client

Open the vault, then go to **Settings → Server**. **Add entry** next to Claude
Desktop or ChatGPT writes the server entry into that client's configuration and
leaves everything else in the file untouched. **Run health check** performs a
real MCP handshake against the vault, which confirms a client will be able to
connect. Restart the client afterwards so it picks up the new server.

For any other MCP client, use one server process per vault, over stdio. Give the
absolute path to the bundled binary, because GUI clients don't inherit your
shell's `PATH`:

```json
{
  "mcpServers": {
    "heimdall": {
      "command": "/Applications/Heimdall.app/Contents/MacOS/heimdall",
      "args": ["mcp", "--vault", "/Users/you/Documents/my-vault"]
    }
  }
}
```

For example, with Claude Code:

```bash
claude mcp add heimdall -- /Applications/Heimdall.app/Contents/MacOS/heimdall mcp --vault ~/Documents/my-vault
```

### What an agent can do

An agent gets two tools, **`read`** and **`write`**, the same two verbs as the
shell.

- **No general file access.** There is no general file read, write, or execute
  tool.
- **No lock tools.** There is no `lock` or `unlock` tool.
- **No way to change vaults.** No tool takes a vault path. The vault is fixed by
  `--vault`, so a call cannot name another vault, switch to one, or discover
  one. Creating a vault is a shell command.

The server's initialization instructions tell a client to:

- start with `read`,
- pass the revision from its latest read when it writes,
- treat a locked path as read-only until you unlock it. A refused write is
  reported to you, with the lock responsible, rather than worked around.

Each tool publishes an input and output schema derived from the same Rust types
the shell returns, so the two cannot drift. Results come back as typed
`structuredContent`. An expected refusal is a tool result flagged `isError`,
carrying a domain `code` (`LOCKED`, `REVISION_CONFLICT`, …), a `message`, and
safe `details`. Protocol errors are reserved for calls the server cannot execute
at all.

Heimdall is pinned to `rmcp` 3.1.3 and MCP protocol `2025-11-25`. To check:

```bash
heimdall --version --json
```

## Locks

A locked note can be read but not written, moved, or deleted, whether by an
agent, a script, or the desktop editor. Every read reports whether a path is
locked. Locking a folder locks everything in it, and a single note inside it can
then be unlocked again.

Only people set locks, at the shell or in the desktop. An agent can see a lock
in every `read`, but it can neither lift one nor set one. Setting one is
withheld too, because a folder lock would erase the note-level unlocks beneath
it.

Against an agent whose only way into the vault is this MCP server, a lock is a
real boundary. It is not one against an agent that also has your shell or
filesystem, because that agent can edit the files directly. Keep such tools away
from a vault you need protected.

## The desktop app

The app has three panes.

- **Left** is the whole vault. Its toolbar has new note, new folder, sort,
  collapse all, and a lock for the whole vault. Right-click an item to rename,
  delete, lock, or unlock it. Locked notes and folders carry a small lock glyph.
- **Middle** is the open note.
  - At the top are a breadcrumb with back and forward, a lock toggle, and a
    switch between the Markdown source and a rendered preview.
  - The preview shows YAML frontmatter as a properties table, draws `mermaid`
    diagrams, and lists linked mentions.
  - A locked note opens read-only. The note's heading is its filename: editing
    one renames the other.
- **Right** is a force-directed graph of the vault's links.
  - Pan with inertia, and zoom about the pointer.
  - Drag a node and its neighbours follow.
  - Click a node to open its note.
  - Hover over a node to light up its neighbourhood and dim the rest.

`⌘O` finds a note by name.

Renaming or moving a note asks before it updates the `[[wikilinks]]` that point
at it. The dialog names the notes it would rewrite, and offers **Update links**
or **Rename only**. Deleting moves a note into the vault's `.trash/`; nothing is
ever unlinked.

The editor saves through `write`, exactly as an agent does, so a lock or a stale
revision refuses it too.

**Settings** (`⌘,`) has four sections:

- **Vault** creates a vault or opens an existing folder.
- **Server** connects AI clients, as described above.
- **Appearance** overrides the system theme and sets an accent colour for each
  theme.
- **Diagnostics** reports which binary is in use, its versions, the vault's
  state, the enforced limits, and recent failures.

Dark mode is pure black behind white, and light mode is the exact inverse. Until
you pick an accent, the theme is monochrome.

## Using the command line

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

From anywhere else, name the vault with `--vault`. Paths are then relative to
the vault root, and the full path to a note works too.

- **Output.** Every command prints a versioned JSON envelope on stdout.
- **Input.** Markdown content always arrives on stdin, never as a
  shell-interpreted argument.
- **Exit codes.** `0` is success. `1` is a domain error, and the error envelope
  is still printed. `2` is a usage error.

## Updating and uninstalling

**To update,** download the new DMG and drag Heimdall into Applications,
replacing the old copy. Your vaults, locks, and client configurations are kept,
and the bundled CLI is replaced along with the app.

**To uninstall:**

1. Quit Heimdall and move it from Applications to the Trash.
2. Remove the CLI link if you made one:
   `sudo rm /usr/local/bin/heimdall`.
3. Optionally, remove Heimdall's own data:
   `~/Library/Application Support/heimdall/`. This holds the registry of known
   vaults, the lock rules, and the write locks. Deleting it unlocks every note.
4. Remove the `heimdall` entry from any AI client you connected.

Your vaults are ordinary folders of Markdown, and uninstalling never touches
them.

## Building from source

You need Rust 1.85 or later, Node.js 22, and Xcode's command line tools.

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

cd apps/desktop
npm install
npm test
npm run sidecar        # build the CLI and stage it as a Tauri externalBin
npm run tauri:dev      # run the desktop app
npm run tauri:build    # an unsigned .app and .dmg
```

`cargo run -p heimdall-cli -- <args>` runs the CLI without installing it.

## Releasing

A release is a Developer ID–signed, notarized, and stapled `Heimdall.app` inside
a signed, notarized, and stapled DMG, for Apple Silicon.

### One-time setup

1. **Signing certificate.**
   - In Keychain Access, choose **Certificate Assistant → Request a Certificate
     from a Certificate Authority**, and save the request to disk. This creates
     the private key in your login keychain.
   - At [developer.apple.com](https://developer.apple.com/account/resources/certificates/list),
     create a **Developer ID Application** certificate from that request. This
     needs the Account Holder role.
   - Download the certificate and double-click it to install it.
   - Check it with `security find-identity -v -p codesigning`. If it shows as
     untrusted, install Apple's *Developer ID - G2* intermediate from
     [apple.com/certificateauthority](https://www.apple.com/certificateauthority/).
   - Export the certificate together with its private key as a `.p12` backup,
     and keep it out of the repository.
2. **Notarization key.**
   - In App Store Connect, go to **Users and Access → Integrations → Team
     Keys** and create a key with the *Developer* role.
   - Download `AuthKey_<KEYID>.p8`. Apple only lets you download it once.
   - Note the key ID and the issuer ID.

### Each release

1. Bump `version` in all three places: `Cargo.toml`,
   `apps/desktop/package.json`, and `apps/desktop/src-tauri/tauri.conf.json`.
2. Set the four `APPLE_*` variables and run the release:

   ```bash
   export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
   export APPLE_API_KEY="<key id>"
   export APPLE_API_ISSUER="<issuer id>"
   export APPLE_API_KEY_PATH="$HOME/.appstoreconnect/private_keys/AuthKey_<KEYID>.p8"

   cd apps/desktop
   npm run release:mac
   ```

The script does the following:

1. Checks the credentials.
2. Builds the release CLI and the app. Tauri signs the app and the bundled CLI
   with the hardened runtime, notarizes the app, and staples the ticket.
3. Signs, notarizes, and staples the DMG.
4. Checks every signature the way Gatekeeper will.
5. Prints the DMG's path and its SHA-256 for the release notes.

If the build fails while creating `Assets.car`, run `killall ibtoold` and try
again. A stale `actool` daemon intermittently crashes on Icon Composer icons.

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

The app never looks for a `heimdall` on your `PATH`; it always runs the
version-matched CLI bundled inside it. It never uses a shell either. React
cannot name an executable or invent a flag: it calls a Tauri command whose
subcommand and argument keys are both allowlisted in Rust.

`docs/SPEC.md` is the source of truth for product behaviour.

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md). Every
contributor signs the [Contributor License Agreement](CLA.md) once, by
commenting on their first pull request.

## License

Heimdall is **source-available, not open source**. It is © 2026 Sam Larsen and
licensed under the [Sustainable Use License 1.0](LICENSE).

**You may:**

- read the source, and modify it;
- use Heimdall, modified or not, for your own internal business purposes, or
  for personal or non-commercial use;
- share it with others free of charge for non-commercial purposes.

**You may not, without a separate license:**

- sell Heimdall, or a version of it;
- offer it to others commercially, for example as a paid or hosted product.

**You must** keep the license and copyright notices intact, and mark any
modified copy you share as modified.

For a commercial license, contact samhlarsen@proton.me.
