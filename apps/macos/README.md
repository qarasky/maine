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

## Test

```sh
swift test
```

The suite uses [Swift Testing](https://developer.apple.com/documentation/testing),
which is built into Xcode 16 and later. It runs a real child process through a
pseudo terminal, so it needs a normal user session — not a headless one.

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
├── scripts/                 build-app.sh, install.sh, render-*.swift (icon pipeline)
├── Sources/Maine/
│   ├── App/                 @main entry point, menu commands, menu bar item, window state
│   ├── Models/              Host, the versioned configuration, the store
│   ├── Services/            ssh command building, ~/.ssh/config reading
│   ├── Sessions/            PTY sessions and the session manager
│   ├── Sync/                sync profiles, engine, transport, coordinator
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
  tabs keeps scrollback and focus intact.
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

## Where things are stored

| What | Where |
| ---- | ----- |
| Saved hosts, passwords, session snapshots | `~/Library/Application Support/Maine/passwords.maine` (encrypted) |
| Remembered local vault key | macOS login Keychain |
| Sync tokens | macOS login Keychain, service `Maine Sync`, keyed by profile ID |
| Opt-in debug logs | `~/Library/Logs/Maine/debug.jsonl` and `debug.previous.jsonl` |
| Keys, known hosts, ssh options | `~/.ssh`, untouched by Maine |

Share both host and password files, and send the master password separately.
The recipient unlocks the copied vault; the Keychain key is not shared. The app
has no sandbox, because it has to run `ssh` and
read the user's `~/.ssh/config`.
