# Architecture

Maine is a monorepo with three clients so far: macOS (shipped), Windows and
Android (both in progress). This document explains how the macOS app is put
together, which decisions are load-bearing, and how the other clients map
them. The Windows layout is documented in
[`apps/windows/README.md`](../apps/windows/README.md), the Android layout in
[`apps/android/README.md`](../apps/android/README.md).

## Why a monorepo

The stated future is four clients — macOS now, Windows and Android in
progress, Linux later.
What those clients share is not code, it is **one file format**: the saved host
list. So the repository splits along that line:

```
maine/
├── apps/macos/          the macOS client: Swift, SwiftUI, AppKit
├── apps/windows/        the Windows client: C#, WPF, ConPTY, WebView2 + xterm.js
├── apps/android/        the Android client: Kotlin, Compose, sshj
├── packages/schema/     the shared, language-neutral configuration format
├── docs/                this document
└── README.md
```

`packages/schema` contains JSON Schema, documentation, examples and a validator —
not a library. Any client, in any language, can implement it without taking a
dependency. That is deliberate: a shared Swift package would be useless to an
Android client, and a shared code layer would need a build system that spans four
toolchains. The format is the contract.

`apps/linux` is not created yet. An empty placeholder would suggest
progress that does not exist.

### The Windows mapping

The Windows client keeps the same architecture and the same byte shapes, with
platform pieces swapped:

| macOS | Windows |
| ----- | ------- |
| `forkpty` + `ssh` child | ConPTY + `ssh.exe` child (`ConPtySession`) |
| SwiftTerm terminal view | xterm.js in one WebView2 per session, all mounted, one visible |
| `~/Library/Application Support/Maine/passwords.maine` | `%APPDATA%\Maine\passwords.maine` (user-only ACL instead of `0600`) |
| Login Keychain session key | DPAPI-protected key file, deleted on lock/quit |
| `SSH_ASKPASS` = app bundle executable | `SSH_ASKPASS` = `Maine.exe` (same helper protocol) |
| `~/.ssh/config` | `%USERPROFILE%\.ssh\config` (same reader, same `Include` rules) |

The vault file is byte-compatible across all three clients: same PBKDF2-SHA256
(600,000 iterations), same AES-256-GCM additional data
(`maine-vault:{version}:{UPPERCASE-UUID}:{base64-salt}:{iterations}`),
same nonce|ciphertext|tag layout, uppercase UUIDs, dates as seconds since
1970. `Maine.Core` builds as plain `net8.0` with no UI dependencies so its
test suite runs on any OS; only `Maine.App` needs Windows.

## What a connection actually is

Maine does not implement SSH, and it does not link a library that does. A session
is:

```
SwiftUI view
   └── TerminalSession                  owns one NSView and one state machine
        └── SwiftTerm.LocalProcessTerminalView
             └── forkpty(3)             pseudo terminal
                  └── /usr/bin/ssh -p 2222 -l deploy example.com
                       └── ~/.ssh/config, ~/.ssh/keys, ssh-agent, certificates
```

Consequences worth stating out loud:

* Everything a user has already configured — `IdentityFile`, `ProxyJump`,
  `Match`, `ForwardAgent`, `ServerAliveInterval`, certificates, hardware keys —
  keeps working, because the user's own `ssh` does the work.
* Maine optionally stores passwords in an encrypted portable vault. If a host
  has a saved password, OpenSSH uses the app's askpass helper to retrieve it.
  Without a saved password, OpenSSH prompts inside the terminal as usual.
* There is no second authentication path to keep in sync, and no private key
  parser to get wrong.

### The command line Maine builds

[`SSHTarget`](../apps/macos/Sources/Maine/Services/SSHTarget.swift) is the only
place that decides what to pass to `ssh`, and it passes as little as possible:

| Host field | What Maine does |
| ---------- | --------------- |
| `hostname` | Always the final argument. An IP, a DNS name, or a `Host` alias from `~/.ssh/config`. |
| `port` = 22 | Nothing. OpenSSH (and therefore `~/.ssh/config`) decides the port. |
| `port` ≠ 22 | `-p <port>` |
| `username` = "" | Nothing. OpenSSH decides, from `~/.ssh/config`, the key, or the local user. |
| `username` ≠ "" | `-l <username>` |

The connection header displays the endpoint as selectable text. Its help tooltip
shows the exact SSH command line without letting a long executable path dominate
the header.

`SSHTarget.Resolution` additionally works out what OpenSSH will *actually*
connect to, by looking the host up in `~/.ssh/config`. This is for display only;
it is best-effort by design, because OpenSSH's real resolution rules (wildcards,
`Match`, `Host *` blocks) are more than a sidebar needs to model.

### The environment

The child `ssh` process inherits the app's environment — not a curated subset —
and only `TERM`, `COLORTERM` and `LANG` are set or defaulted. Changing
`SSH_AUTH_SOCK` or `PATH` would break ssh-agent, proxies and `ProxyCommand`, so
Maine leaves them alone.

For hosts with saved passwords, Maine additionally sets `SSH_ASKPASS` to its own
executable and `SSH_ASKPASS_REQUIRE=force`, with a helper-mode flag and the host
ID. No secret is passed in process arguments or environment variables. The
helper-mode entry point runs before SwiftUI starts, reads the local unlock key,
and decrypts that host's password. Passphrase, MFA and host-trust prompts are
handled separately in a dialog; host keys are never automatically accepted.

## The macOS app

Five layers, one file each, each testable without the others:

| Layer | Files | Responsibility |
| ----- | ----- | -------------- |
| App | `App/MaineApp.swift`, `App/AppState.swift`, `App/MenuBarController.swift` | Entry point, menu commands, the menu bar item that keeps the app resident, and the window-level UI state the menus and views share. |
| Models | `Models/Host.swift`, `Models/HostConfiguration.swift`, `Models/HostStore.swift` | The host, the versioned file format, and the store that owns the file on disk. |
| Services | `Services/SSHTarget.swift`, `Services/SSHConfigFile.swift` | Building the `ssh` invocation, and reading (never writing) `~/.ssh/config`. |
| Sessions | `Sessions/TerminalSession.swift`, `Sessions/SessionManager.swift` | One PTY session per host, and which sessions are open. |
| Views | `Views/*.swift` | SwiftUI presentation only. |

Data flows one way: a view asks `SessionManager` to open a host, the manager
creates a `TerminalSession`, which starts the `ssh` child and publishes its
status back through `ObservableObject`. Nothing in `Views` knows how a command
line is built, and nothing in `Services` knows SwiftUI exists.

### Sessions are owned by the session manager, not the views

`TerminalSession` owns its `LocalProcessTerminalView`. The SwiftUI view that
displays a session is a thin `NSViewRepresentable` that hands back that existing
view inside a neutral, constrained `TerminalHostView`. Every open session stays
mounted in a stable `ZStack`, including while no session is selected, so:

* switching between sidebar sessions keeps scrollback, and
* reconnecting creates a whole new session (and view), which is why it starts
  from a clean screen.

Reconnect replaces one published session in place, rather than closing a tab
and briefly publishing an empty workspace. Connection-state changes triggered
by terminal sizing are deferred until after AppKit/SwiftUI layout finishes.

The root split view is explicitly sized to the window's GeometryReader viewport.
The terminal has its own flexible viewport and receives only the space left
after the header and error banner. Its previous AppKit height must never feed
back into the split view's ideal/fitting height: that previously expanded the
layout beyond the window and moved controls off-screen after an error exit.

Sessions are keyed and selected by session UUID, not host UUID. A regular host
click reveals its selected/existing connection; New Session explicitly creates
another independent PTY for that host. When a host has multiple sessions, the
sidebar renders numbered, indented child rows. Selecting, reconnecting or closing
a child targets only its session UUID. Reconnect preserves its display number;
deleting a saved host closes all of its sessions. Closing a selected child prefers
another session of that host, then the last remaining session of another host.

`SessionManager` keys sessions by host id, so opening an already-connected host
brings its terminal back rather than opening a second connection.

### Status without lying

`TerminalSession.State` has six cases: idle, connecting, connected, closed(exit
code), disconnected(exit code) and failed(message). An intentional disconnect
records the raw process exit code but does not show an error banner, even when
OpenSSH exits with 255 in response to termination. Unexpected non-zero exits
still show their error banner. The transition from *connecting* to *connected* is
driven by the first layout of the terminal view, not by a guess: that is the
moment the PTY has a real size and `ssh` has a terminal to talk to. A session
that ends with a non-zero exit code raises a banner with the exit code, and the
terminal above it keeps whatever `ssh` printed.

### Storage

`HostStore` reads and writes hosts through `PasswordStore` in the single active
`~/Library/Application Support/Maine/passwords.maine` vault:

* Reading a missing file is not an error — it is an empty host list.
* A file that cannot be parsed, or that has a higher `schemaVersion`, is reported
  to the user and left exactly as it was. Maine does not "fix" or overwrite data
  it does not understand.
* Saving is atomic and the file is written `0600`.

Version 2 contains a vault UUID, 32-byte random salt, PBKDF2-SHA256 iteration
count (600,000), encrypted verification marker, and one encrypted JSON payload
containing hosts and passwords. IDs and metadata are not visible outside the
ciphertext. AES-256-GCM authenticates the version, vault ID, salt and iteration
count as additional data. Host edits/deletes commit metadata and credentials in
one atomic replacement. Locked host lists are empty and cannot be decrypted. Each
write uses fresh nonces and atomic replacement, with `0600` file permissions.
The master password is never stored. The derived encryption key is remembered
in the local login Keychain, indexed by vault ID, only for the current app
session. Normal quit removes it; launch resets it even after a crash. Every
launch presents the master-password sheet, and locked editors disable password
input. While the process runs, Maine is a menu bar resident: closing the window
only hides it, so the vault stays unlocked and sessions stay connected until
quit or an explicit lock. A password is required for adding or saving a host;
existing hosts without one show an actionable edit prompt when connection is
attempted.
Copying only the vault to another Mac requires its
master password to unlock; neither Keychain export nor plaintext export is needed.

Version-1 credentials and legacy `hosts.json` migrate only on successful unlock.
Original files are retained for recovery (`passwords.maine.v1-backup` and the
old plaintext `hosts.json`); users should remove/archive the latter after verifying
migration. Subsequent unlocks never re-import stale metadata. Explicit JSON URLs
in `HostStore` remain a legacy test/tooling adapter, not the application's storage.
There is no host-import UI; OpenSSH still reads its own configuration normally.

The encrypted payload has an optional `sessions` array so earlier version-2
vaults remain readable. Each saved snapshot contains an independent UUID, host
UUID, display name, plain-text terminal history and save date. Sessions checkpoint
automatically on creation, every two seconds while unlocked, and before normal
quit or vault lock. History is capped at 1 MiB from SwiftTerm's retained buffer.
Unchanged snapshots are not rewritten. Explicit close deletes the encrypted
record before stopping/removing the runtime session; a failed delete keeps the
session visible and reports an error. Quit checkpoints and stops processes without
deleting their records. While locked, the last checkpoint is retained and closing
requires unlocking. Deleting a host deletes its records in the same atomic vault write.

After unlock, saved snapshots restore their history as scrollback and reconnect
automatically. Repeated restores skip already-mounted saved IDs. Restored output is fed only
to the renderer, with control characters stripped; it is never sent to a PTY
or replayed as commands. Reconnect preserves the custom name and saved-copy
identity. Snapshot output and custom names are never added to debug logging.
Animations affect sidebar grouping and opacity, never terminal ownership; they
respect accessibility Reduce Motion.

### Deliberate UI choices

* **Single click connects.** The sidebar lists all hosts and sessions. There is
  no All Hosts page, host-details page or separate tab strip. Each open session
  has a sibling close button that disconnects
  without deleting its saved host or activating the row being closed.
  Double-clicking a session name starts inline rename (Return commits, Escape
  cancels). One shared row background paints
  both hover and selection, including padding and the close control.
* **Port sits beside IP / Hostname.** It defaults to 22, without a separate
  label or explanatory text. Username can still defer to `~/.ssh/config`.
* **Content outranks chrome.** A compact connection header shows the endpoint
  and a quiet status indicator; the exact command is available in its help tooltip.
  Native search supports Command-F; Delete closes a session rather than deleting
  its host. See [the design references and layout decisions](macos-design.md).
* **Resident in the menu bar.** The status item is the app's only presence:
  there is no Dock icon (`LSUIElement`). Left-click restores the window;
  right-click offers Open, Lock or Unlock, and Quit. Quit checkpoints sessions
  and removes the vault key exactly like Command-Q.
* **Copy Credentials.** **Host → Copy Credentials** and a host's context menu
  write `[ip]`, `[port]` (only when not 22), `[username]` (when set) and
  `[password]` (when a saved password is unlocked) to the pasteboard as plain
  text. A locked vault requests the master password first; the debug log
  records the host ID only, never the block, and the text stays on the
  clipboard until something else replaces it.
* **State lives where it is used.** The host list is owned by `HostStore`, open
  sessions by `SessionManager`, and the host being edited by `AppState`, so the
  menu commands and the views can never disagree about what is selected.

## The Android client

Kotlin + Jetpack Compose in `apps/android`, following the same five layers
(models, SSH parameters, vault-backed store, sessions, views) without sharing
code: the contract between clients is the file format, not a library.

Three platform differences are load-bearing:

* **No system `ssh`.** Android cannot fork `/usr/bin/ssh`, so the transport
  is the sshj library instead of a PTY child process. `SshTarget` keeps the
  macOS display rules (port 22 omits `-p`, empty user omits `-l`) and now
  also carries the dial parameters (hostname, port, username) the transport
  uses. The SSH layer stays behind the `SshConnection` interface, so the UI
  and session lifecycle do not change when the transport lands.
* **The vault file is portable.** `VaultStore` implements the macOS version-2
  envelope byte-for-byte — same JSON fields, same PBKDF2-SHA256 parameters
  (600,000 iterations, 32-byte salt), same AES-256-GCM nonce layout and the
  same additional authenticated data — so `passwords.maine` unlocks on
  either client with the same master password. It lives in app-private
  internal storage (`filesDir/passwords.maine`); the derived key is held in
  memory only and every launch starts locked. Version-1 vaults are rejected:
  unlock once on macOS to migrate. `VaultGoldenTest` unlocks a fixture
  generated by the Swift implementation itself, which is what proves the
  compatibility rather than any amount of eyeballing.
* **UUID casing.** macOS writes uppercase `UUID().uuidString` and looks
  passwords up with that exact casing. Android normalizes IDs to uppercase
  on read and write, and compares case-insensitively everywhere else.

Session semantics mirror macOS: sessions keyed by session UUID, explicit
close deletes the encrypted record first (failure keeps the session
visible), backgrounding checkpoints scrollback like a quit, restore
reconnects automatically with history as scrollback (never replayed). The
terminal UI is deliberately a stub for now — scrollback text plus an input
row — with full terminal emulation replacing only the output view later.

## Testing strategy

Tests are written with Swift Testing and run with `swift test`. They cover the
parts where being wrong is expensive:

* the file format, including refusing a newer version and reporting a readable
  message for a bad field;
* host validation;
* persistence, restart behaviour and file permissions;
* the exact `ssh` argument lists, including "port 22 means say nothing";
* the `~/.ssh/config` reader, including `Include`, comments and wildcards;
* the PTY session lifecycle — start, receive output, exit with a code, and
  disconnect — using a real child process;
* visible, full-window layout after both an unexpected exit 255 and an intentional
  disconnect returning 255, including resizing. These tests assert that the
  split view fits within the window and that the header, banner and terminal
  stay on-screen, rather than merely checking view attachment.

The last group runs a real `forkpty` and a real `/bin/echo`, so the most
failure-prone part of the app is covered without needing a network.

## Known limitations

* One window. Sessions are managed in the sidebar, not in separate windows.
* `SSHTarget.Resolution` models exact `Host` matches only; wildcard and `Match`
  blocks in `~/.ssh/config` are not resolved for display (OpenSSH still applies
  them at connect time).
* No port forwarding UI, no SFTP, no key management, no known-hosts editing.
  Those are all handled by `~/.ssh/config` today, and adding a second place to
  configure them would be a step backwards.
