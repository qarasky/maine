# Maine for macOS

A SwiftUI application in a single Swift package. It has one dependency,
[SwiftTerm](https://github.com/migueldeicaza/SwiftTerm), which provides the
terminal emulator; everything else is the standard library, AppKit and SwiftUI.

## Requirements

* macOS 14 or later
* **Xcode 16 or later.** SwiftUI's property-wrapper macros (`@State`,
  `@Environment`, …) are implemented by a plugin that ships with Xcode, so a
  Command Line Tools–only install can compile the package but not the views.
  Check with `xcode-select -p`; it should point inside
  `/Applications/Xcode.app/Contents/Developer`.
* `/usr/bin/ssh` (ships with macOS)

## Build

```sh
scripts/build-app.sh --release          # build/Maine.app
scripts/build-app.sh --release --run    # build and launch
scripts/build-app.sh --debug            # debug configuration
```

The package produces a plain executable, so the script wraps it in the bundle
structure macOS expects and ad-hoc signs it. Without the bundle, launching the
app works but window and menu behaviour is not what a user expects.

`scripts/install.sh` builds a release app and copies it to `/Applications`.

During development, `swift build` and `swift run Maine` are usually enough.

## Install and update

Download `Maine-X.Y.Z.dmg` from
[GitHub Releases](https://github.com/qarasky/maine/releases) and drag
`Maine.app` onto
`/Applications`. The app is ad-hoc signed and not notarized, so the first launch
of a downloaded copy may be blocked: right-click it in Finder and choose
**Open**, or run `xattr -dr com.apple.quarantine /Applications/Maine.app`.

Maine updates itself from the same releases. While the vault is unlocked it
checks at unlock and then every two hours, and **Maine → Check for Updates…**
checks on demand; **Settings → Updates** holds the automatic toggle (default on)
and **Check Now**. A newer release offers **Update**, **Later** and **Skip This
Version**. **Update** downloads the DMG, verifies the SHA-256 published in the
same release, copies the new bundle beside the running one, swaps it from a
detached helper after the app quits and relaunches. Builds without `--version`
report `0.0.0-dev` and never auto-check. The check is one GET to GitHub's
Releases API with no identifiers; integrity is HTTPS plus SHA-256, not proof of
authorship. [docs/releasing.md](../../docs/releasing.md) covers cutting a
release and the Gatekeeper caveat.

```sh
scripts/build-app.sh --release --version 1.2.3   # stamp a version
scripts/make-dmg.sh 1.2.3                        # build/Maine-1.2.3.dmg + .dmg.sha256
```

## Test

```sh
swift test
```

The suite uses [Swift Testing](https://developer.apple.com/documentation/testing),
which is built into Xcode 16 and later. It runs a real child process through a
pseudo terminal, so it needs a normal user session — not a headless one.

Plain `swift test` skips the window and layout tests, which create, show,
activate and resize real windows. Run those with `MAINE_UI_TESTS=1 swift test`.
`SyncServerIntegrationTests` runs only when `MAINE_SYNC_SERVER_BIN` points at a
built `maine-sync-server`.

The schema package is checked separately and needs only Python 3:

```sh
(cd ../../packages/schema && python3 tools/validate.py)
```

## Layout

```
apps/macos/
├── Package.swift            SwiftPM manifest
├── Package.resolved         Pinned dependency versions
├── Resources/Info.plist     Bundle metadata used by scripts/build-app.sh
├── scripts/                 build-app.sh, make-dmg.sh, install.sh, render-*.swift (icon pipeline)
├── Sources/Maine/
│   ├── App/                 @main entry point, menu commands, menu bar item, window state
│   ├── Agents/              MCP bridge and server, tool handling, access rules, activity log
│   ├── Models/              Host, the versioned configuration, the store
│   ├── Services/            vault, ssh command building, ~/.ssh/config reading, debug logs
│   ├── Sessions/            PTY sessions and the session manager
│   ├── Sync/                sync profiles, engine, transport, coordinator
│   ├── Update/              semver, update feed, installer, update coordinator
│   └── Views/               SwiftUI views
└── Tests/MaineTests/        Swift Testing suites
```

`docs/architecture.md` at the repository root explains how the pieces fit
together and why they are arranged that way.

## Notes for contributors

* **Platform-specific code stays here.** Shared formats live in
  `packages/schema`, shared code does not exist yet.
* **Maine is a menu bar resident.** Closing the window hides it while sessions
  keep running; the status item restores the window (left-click) and offers
  Lock, Unlock and Quit (right-click). `applicationShouldTerminateAfterLastWindowClosed`
  must stay false, and quitting goes through the same checkpoint path as ⌘Q.
* **Keep the terminal view alive.** Terminals are owned by their
  `TerminalSession`, not by the SwiftUI view that displays them, so switching
  sessions keeps scrollback and focus intact. Every session's view is a
  `CapturingTerminalView`, which passes each PTY byte to the session before
  rendering it, so agent output capture also works for sessions the user
  opened.
* **Host passwords are optional.** Leave the password empty for key-based
  login: Maine runs plain `ssh`, so keys in `~/.ssh`, `ssh-agent` and
  `~/.ssh/config` keep working. A saved password is used only when ssh asks
  for one.
* **Do not add a second way to reach a host.** Everything that talks to a server
  goes through the system `ssh` binary.
* **Xcode is required to build.** Not for the app's own code, but for SwiftUI's
  macros; see Requirements above.

## Sync

Optional and self-hosted: **Settings → Sync** (⌘,) creates or joins a slot on a
sync server you run (`apps/sync-server`). While the vault is unlocked, the
active profile pulls on unlock, pushes two seconds after an edit, polls with
`HEAD` every 30 seconds, and makes a best-effort push on lock or quit.
Conflicts never lose data: the losing side is kept as
`passwords.conflict-….maine` before the server's copy is adopted. The wire
contract is `docs/sync-protocol.md`; the client checklist is
`docs/sync-client.md`.

## Agents (MCP)

Off by default: **Settings → Agents** (⌘,) turns the local MCP server on, pairs
clients, and chooses the groups and hosts agents may use on this Mac. A client
starts `Maine --mcp`, a stdio bridge to the running app over a Unix socket that
only this user can open. Each client gets a revocable token; only its hash is
stored in Keychain (service `Maine Agents`). Saved passwords are typed into
detected prompts and never returned, and a session the user opened can be
handed to a client. The activity log is encrypted and device-local. The
protocol, tools and threat model are in `docs/mcp.md`.

## Where things are stored

| What | Where |
| ---- | ----- |
| Saved hosts, groups and passwords | `~/Library/Application Support/Maine/passwords.maine` (encrypted) |
| Session snapshots, collapsed groups, agent access rules and activity log | `~/Library/Application Support/Maine/sessions-<vault id>.maine` (encrypted, device-local, never synced) |
| Remembered local vault key | macOS login Keychain, service `app.maine.vault-key` |
| Sync tokens | macOS login Keychain, service `Maine Sync`, keyed by profile ID |
| MCP client token hashes | macOS login Keychain, service `Maine Agents`, keyed by client ID |
| Update preferences (automatic checks, skipped version) | `UserDefaults`, keys `app.maine.update.*` (never in the vault, never synced) |
| Opt-in debug logs | `~/Library/Logs/Maine/debug.jsonl` and `debug.previous.jsonl` |
| Keys, known hosts, ssh options | `~/.ssh`, untouched by Maine |

Share the vault file, and send the master password separately.
The recipient unlocks the copied vault; the Keychain key is not shared. The app
has no sandbox, because it has to run `ssh` and
read the user's `~/.ssh/config`.
