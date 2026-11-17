# Maine for Linux — design

Status: draft for approval (2026-11-17). Nothing is built yet.

## Intent

Skyler runs a KDE Plasma 6 / Wayland workstation (Kubuntu) and wants Maine
there: the same local-first SSH client, same vault file, same agent access, as a
native-feeling app. The audience is Skyler's own machines; this is not a
distribution project. Skyler chose Rust + Qt 6 (QML) over Tauri because a
web-shell Linux client next to a native Swift macOS app would not make sense:
each client should be native to its platform.

This is a **rewrite in a new stack**, not a port. Swift/SwiftUI/AppKit/Keychain/
SwiftTerm do not exist on Linux. What carries over is the contracts: vault v2
envelope, `packages/schema`, the sync protocol (server is Rust and already runs
on Linux), the MCP tool surface, and the "system `ssh` on a PTY" architecture.

## Goals

1. `apps/linux`: a Cargo workspace producing one binary, `maine`, with the
   same modes as macOS: GUI, `--askpass`, `--mcp`.
2. Open and write the **same `passwords.maine`** (byte-compatible vault v2; a
   vault copied between macOS, Android and Linux unlocks with the same master
   password). Proven by the Swift-generated golden file
   (`apps/android/app/src/test/resources/vault-golden-v2.json`).
3. Feature parity with macOS, delivered in phases (below): hosts/groups, real
   terminal sessions on the system `ssh`, saved passwords via askpass, session
   restore, sync client, MCP agent access.
4. Looks and behaves like a Plasma app (Qt Quick Controls with the system
   style, system tray via StatusNotifier, respects reduced motion setting).
5. Honors every invariant in `AGENTS.md` that is not macOS-specific (see
   *Invariants on Linux*).

## Non-goals

* Other desktops/compositors as a target (GNOME tray, X11, Flatpak, deb/rpm,
  AppImage). It should run, but is only tested on Plasma/Wayland.
* A self-updater. The Linux client is built from source (`scripts/install.sh`
  installs the binary and a `.desktop` file). Invariant 13 (updater) is
  macOS-only; Linux makes **no automatic network calls besides sync**.
* Windows, a shared Rust/Qt code layer with other clients, new vault features,
  key management, a hosted service.
* Touching the macOS or Android clients, except docs and one new shared test
  fixture if needed.

## Anti-goals — it fails if

* The vault byte format or the schema diverges from macOS in any way.
* A password, token or vault key ever reaches an argument list, environment
  variable, log or diagnostics line (invariant 8).
* Switching sessions reparents/restarts a terminal, or a long error/banner can
  grow the layout beyond the window (invariants 5, 6).
* Tests open real windows on Skyler's screen.

## Constraints

* Rust (cargo 1.100 present). Qt 6.11 runtime libs are installed; **dev
  packages are not** (`qt6-base-dev`, `qt6-declarative-dev`; no GTK or VTE
  needed). Installing them with apt is in scope.
* UI: QML via **cxx-qt 0.10**. C++ is allowed only as a thin shim where cxx-qt
  cannot subclass what Qt needs (expected: one `QQuickPaintedItem`/scene-graph
  item for the terminal grid). Everything else — vault, hosts, ssh, sync,
  agents — is plain Rust with no Qt types.
* Terminal emulation: **`alacritty_terminal` 0.26** owns the screen state for
  rendering and for the agents' `read_screen`; **`portable-pty` 0.9** (or
  `rustix`/`nix` directly) owns the PTY. Verified on crates.io today; versions
  are re-checked when the workspace is created.
* Tray: `ksni` 0.3 (StatusNotifierItem). Secrets: Secret Service via `oo7`
  0.7 (KWallet answers it; this machine's wallet has a blank password on
  purpose).
* Repo rules: commit only when asked; docs travel with behavior
  (`README`, `docs/guide.md`, `docs/development.md`, `docs/architecture.md`,
  `AGENTS.md` table and commands).
* Tests must never take over the screen: window/layout tests run with
  `QT_QPA_PLATFORM=offscreen` and are tagged so the default `cargo test` skips
  them unless offscreen is guaranteed.

## Approaches considered

| Option | Trade-off | Outcome |
| ------ | --------- | ------- |
| Tauri + xterm.js | Best terminal, easiest UI, webview look; makes native macOS app pointless | rejected by Skyler |
| GTK4 + libadwaita + VTE | Least terminal work; looks foreign on Plasma; no tray API | rejected |
| **Qt 6 + QML via cxx-qt** | Native on KDE, native tray; the terminal grid renderer must be written | **chosen by Skyler** |
| Pure Rust (egui/iced) | No system deps; hand-built selection/IME/rendering, least native | rejected |

Within Qt: Rust core + QML UI + one small C++ terminal item. Fallback if the
spike fails: wrap `qtermwidget6` (C++) and keep a parallel `alacritty_terminal`
only for agents — worse, so used only if the custom item is not viable.

## Architecture

```
apps/linux/
├── Cargo.toml                 workspace
├── crates/maine-core/         no Qt, no UI; fully unit-testable
│   ├── vault.rs               v2 envelope (PBKDF2-SHA256 600k, AES-256-GCM, AAD)
│   ├── model/                 Host, HostGroup, HostLayout, SavedSession, validation
│   ├── ssh/                   SshTarget (command line), SshConfigFile (read-only)
│   ├── session/               PTY + emulator wrapper, session state machine
│   ├── sync/                  profiles, engine, transport (HTTP), coordinator
│   ├── agents/                protocol, socket server, tools, driver, OSC 133,
│   │                          output buffer, access rules, audit log
│   └── secrets.rs             Secret Service (vault key, sync tokens, MCP token hashes)
├── crates/maine-app/          cxx-qt bridge + QML + the C++ terminal item + tray
│   ├── src/                   QObjects exposing core state to QML
│   ├── qml/                   Sidebar, SessionView, HostEditor, Settings, Unlock
│   └── cpp/TerminalItem.*     draws an emulator grid snapshot; keyboard/mouse/IME
└── scripts/install.sh
```

Data flows the same way as on macOS: QML asks the session manager (Rust) to open
a host; it creates a session that owns a PTY running `/usr/bin/ssh …` built by
`SshTarget` (same table as `docs/architecture.md`); the session publishes state
to QML. A terminal item is created once per session and kept alive in a stable
stack — never reparented.

Where macOS and Linux differ on purpose:

| Concern | macOS | Linux |
| ------- | ----- | ----- |
| UI | SwiftUI + AppKit | QML + Qt Quick Controls (system style) |
| Terminal | SwiftTerm `NSView` | `alacritty_terminal` grid + C++ paint item |
| Vault unlock key for askpass | Keychain, deleted every launch | **Never persisted.** Lives in app memory; the askpass helper asks the running app over a 0600 Unix socket (see *Askpass*) |
| Sync tokens, MCP token hashes | Keychain | Secret Service (`oo7`) |
| Window chrome | hidden title bar | native KWin title bar (see *Window chrome*) |
| Resident app | NSStatusItem | StatusNotifierItem (`ksni`); window hides, process stays |
| Vault path | `~/Library/Application Support/Maine/passwords.maine` | `$XDG_DATA_HOME/maine/passwords.maine` (default `~/.local/share/maine/`), 0600, dir 0700 |
| Agent socket | `~/Library/Application Support/Maine/agent.sock` | `$XDG_RUNTIME_DIR/maine/agent.sock`, 0600 in a 0700 dir |
| Askpass | `Maine --askpass` reading Keychain key | `maine --askpass` asks the running app over a socket; `SSH_ASKPASS_REQUIRE=force` unchanged |
| Preferences | `UserDefaults` | `$XDG_CONFIG_HOME/maine/config.toml` (non-secret only) |
| Updates | tag-driven DMG, in-app updater | none; build from source |
| Reduce Motion | OS setting | KDE animation-speed / `QT_QUICK_CONTROLS` motion off; at minimum one app-level switch |

### Askpass

The vault key is never written anywhere (this is stricter than macOS, which
parks it in the Keychain while unlocked, and avoids leaving a derived key at
rest in a blank-password KWallet). `maine --askpass` is a tiny, UI-less mode:
ssh runs it with `SSH_ASKPASS=<maine>`, `SSH_ASKPASS_REQUIRE=force` and a host
ID (never a secret) in the environment; it connects to the running app's
askpass socket (`$XDG_RUNTIME_DIR/maine/askpass.sock`, 0600, peer uid checked
with `SO_PEERCRED`), sends the prompt text and host ID, and prints the answer.
The app decides: a saved-password prompt is answered from the unlocked vault;
passphrase, MFA and host-trust prompts open a QML dialog in the app (host keys
are never auto-accepted). The app is always running when ssh is, because the app
spawns ssh. If the vault is locked the helper fails closed.

If Secret Service is absent or locked, the GUI still runs; sync and MCP client
tokens are unavailable and Settings says why.

### Window chrome

KWin draws a native title bar on Plasma/Wayland and the client cannot hide it
without faking window controls. Linux keeps the native title bar. Invariant 6 on
Linux therefore reads: no tab bar, no toolbar, no All Hosts page; sidebar-only
navigation with full-row hit targets.

## Phases (each ends with tests green and the app usable)

0. **Spike (throwaway, labelled).** Install Qt dev packages; cxx-qt 0.10
   hello-world on Qt 6.11; a C++ item that paints an `alacritty_terminal` grid
   fed by a PTY running bash. **Go criteria:** interactive vim/htop/tmux render
   correctly; `cat` of a ~100 MB file stays responsive (no UI freeze, scrolling
   fluid); keyboard incl. modifiers and arrows; resize reflows and sends
   SIGWINCH; mouse selection and copy/paste; Cyrillic typing and paste; compose
   and IME through Wayland text-input work at least for dead keys/compose;
   wide characters and emoji render without misaligning columns. **No-go
   means reopening the toolkit choice with Skyler** — not silently switching to
   Qt Widgets or `qtermwidget6`.
1. **Core: vault, model, ssh.** `maine-core` vault (golden-file test against the
   Swift-generated fixture, wrong password / malformed file leave the file
   untouched, atomic 0600 writes), host/group/layout model and validation
   (checked against `packages/schema` examples), `SshTarget`, `SshConfigFile`.
2. **App MVP.** Master-password gate on every launch (invariant 1), sidebar-only
   navigation with groups, search, drag reorder, host editor, terminal sessions
   keyed by session UUID, askpass, encrypted device-local session file
   (`sessions-<vault id>.maine`), close-vs-quit semantics, tray-resident window,
   metadata-only diagnostics with `--debug`.
3. **Sync client.** Per `docs/sync-client.md`: profiles, create/join/pull/push,
   conflict copies, triggers (unlock pull, 2 s debounce, 30 s HEAD poll,
   lock/quit flush), workspaces; integration test against the real
   `maine-sync-server` binary like macOS' `SyncServerIntegrationTests`.
4. **MCP agent access.** Per `docs/mcp.md`: `maine --mcp` bridge, Unix socket,
   token-hashed clients, access rules, tools, OSC 133 completion, hand-off,
   pause/stop, encrypted activity log. The existing wire contract is kept so the
   tool surface is identical across macOS and Linux.

Each phase is planned and built separately (own plan, own review). The first
plan covers phases 0 and 1 in detail; phases 2–4 stay milestones until the spike
result is in, then get their own plans.

## Invariants on Linux

Carried over unchanged: 1 (master password every launch), 2 (one vault file,
session snapshots device-local and sealed), 3, 4 (session UUIDs, close vs quit),
5 (terminals stay mounted, layout bounded by window), 6 (sidebar-only), 8
(diagnostics metadata-only), 10 (resident: window closes, process stays, vault
stays unlocked until Quit or lock), 11 (sync whole-file and opaque), 12 (agent
access opt-in, scoped, local). Adapted: 7 (signing) not applicable; 9 (Reduce
Motion) follows the Plasma setting; 13 (updater) macOS-only, Linux has no
automatic network calls except sync.

## Testing

* `cargo test` for `maine-core` (the bulk): vault golden + roundtrip, model,
  `SshTarget`, config parser, sync engine against an in-process fake and against
  the real server binary, agent output parser and OSC 133, access rules.
* App-level geometry/lifecycle tests run with `QT_QPA_PLATFORM=offscreen`.
* `packages/schema` validator keeps running in CI for shared examples.
* Reproduce-before-fix applies (failing regression test first).

## Decided by Skyler

* Language/toolkit: Rust + Qt 6/QML via cxx-qt.
* Audience: Skyler's own machines.

## My calls (flag any you disagree with)

* Cargo workspace with a Qt-free `maine-core`, so the UI toolkit stays swappable.
* Thin C++ shim for the terminal item; otherwise Rust.
* `alacritty_terminal` + `portable-pty` for emulation/PTY.
* Secret Service via `oo7` for the local vault key, sync tokens and MCP token
  hashes (parity with Keychain use).
* XDG paths as in the table; config in TOML.
* No self-updater, no packaging beyond `install.sh` + `.desktop`.
* Plasma/Wayland only as a tested target.
* Phased delivery 0–4 in the order above.
* The vault key is never persisted; askpass goes through the running app
  (*Askpass*). Stricter than macOS.
* Native KWin title bar; invariant 6 reworded for Linux (*Window chrome*).
* Spike go/no-go criteria as listed; a no-go reopens the toolkit decision.
* Invariant 13 does not apply; no automatic network calls besides sync.

## Left to the builder

Exact Qt Quick Controls structure and styling (reuse `DesktopMetrics` values as
the spacing vocabulary), rendering strategy inside the terminal item (glyph
cache, damage tracking), IME details, `ksni` menu layout, file/module naming
inside the crates, how drag-and-drop reordering is wired in QML, CI wiring.
