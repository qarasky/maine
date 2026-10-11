# AGENTS.md

Instructions for AI coding agents working in this repository. Humans use `README.md`; this file is the contributor shortcut.

## What this is

Maine is a local-first SSH client for macOS (Swift 6, SwiftUI, macOS 14+), Windows (C#, WPF, .NET 8) and Android (Kotlin, Compose, SDK 28+). Swift package at `apps/macos` with one dependency (SwiftTerm); .NET solution at `apps/windows` (`Maine.Core` has no UI dependencies); Gradle project at `apps/android` (sshj for SSH, Compose for UI, and a vendored Termux terminal module for emulation). No backend, no sync, no telemetry. Shared cross-client contract lives in `packages/schema`, not shared code — except the vault envelope, which every client implements byte-for-byte so the file is portable.

```
maine/
├── apps/macos/     The app: Sources/Maine, Tests/MaineTests, scripts/, Resources/
├── apps/windows/   WPF + C# client: src/Maine.Core, src/Maine.App, tests/
├── apps/android/   Kotlin + Compose client: app/src/{main,test}, terminal/, tools/, README.md
├── packages/schema/ Versioned JSON config format + Python validator
├── docs/           architecture.md, macos-design.md, images/
└── README.md
```

## Commands

Run app commands from `apps/macos`:

```sh
swift test                                   # full suite (Swift Testing, ~84 tests)
scripts/build-app.sh --release [--run]       # build/Maine.app, ad-hoc signed
scripts/build-app.sh --debug [--run]         # debug configuration
swift run Maine                               # debug executable directly
build/Maine.app/Contents/MacOS/Maine --debug  # release build with debug logging
(cd ../../packages/schema && python3 tools/validate.py)
```

Tests run a real child process through a PTY and need a normal (non-headless) user session. Xcode 16+ must be selected via `xcode-select`; Command Line Tools alone cannot compile the SwiftUI views.

Run Windows commands from `apps/windows` (user-local .NET 8 SDK works; `Maine.App` needs Windows to run):

```sh
dotnet build src/Maine.Core/Maine.Core.csproj
dotnet test tests/Maine.Core.Tests/Maine.Core.Tests.csproj   # 57 tests, incl. Swift-produced vault fixture
```

Run Android commands from `apps/android` (JDK 17+, Android SDK with platform 36; JVM unit tests need no emulator):

```sh
./gradlew :app:testDebugUnitTest              # JVM unit tests (48 tests)
./gradlew :app:assembleDebug                  # debug APK
swift tools/make-golden-vault.swift <password> app/src/test/resources/vault-golden-v2.json
```

## Architecture map

| Area | Files | Role |
| --- | --- | --- |
| Entry, menus | `Sources/Maine/App/MaineApp.swift`, `App/AppState.swift` | Window state shared by menus and views; master-password gate on every launch |
| Hosts | `Sources/Maine/Models/Host*.swift` | Host model, validation, vault-backed store |
| Vault | `Sources/Maine/Services/PasswordStore.swift`, `LocalVaultKeyStore.swift` | Single active file `passwords.maine` (v2, AES-256-GCM + PBKDF2-SHA256); Keychain holds only the session helper key |
| SSH | `Sources/Maine/Services/SSHTarget.swift`, `SSHConfigFile.swift`, `SSHAskPass.swift` | Builds the exact `ssh` command line; reads `~/.ssh/config`; supplies saved passwords |
| Sessions | `Sources/Maine/Sessions/SessionManager.swift`, `TerminalSession.swift` | Open sessions keyed by session UUID; terminal NSView owned by the session, never reparented |
| Views | `Sources/Maine/Views/` | Sidebar-only navigation; `DesktopMetrics.swift` is the single spacing vocabulary |
| Tests | `Tests/MaineTests/` | Geometry, lifecycle, encryption/migration, persistence regressions |

Windows (`apps/windows`, C# + WPF, .NET 8):

| Area | Files | Role |
| --- | --- | --- |
| Core (no UI) | `src/Maine.Core/` | Host model/validation, versioned format, vault v2 (byte-compatible), ssh command + config reader, snapshots |
| App | `src/Maine.App/` | Sidebar, ConPTY sessions, WebView2 terminals, askpass helper, DPAPI key store |
| Tests | `tests/Maine.Core.Tests/` | Validation, format refusal, ssh args, config reader, vault roundtrip + Swift-generated fixture |

Android (`apps/android`, Kotlin + Compose, minSdk 28):

| Area | Files | Role |
| --- | --- | --- |
| Entry, state | `ui/MainActivity.kt`, `ui/MaineViewModel.kt` | Vault gate on every launch; window-level state shared by screens |
| Hosts | `model/Host*.kt` | Host model, validation, versioned format (schema parity) |
| Vault | `vault/VaultStore.kt` | `passwords.maine` v2, byte-compatible with macOS; key in memory only |
| SSH | `ssh/SshTarget.kt`, `ssh/SshConfigFile.kt`, `session/SshjConnection.kt` | Live sshj transport (password auth, all I/O off the main thread); display args + dial params; read-only ssh config |
| Sessions | `session/SessionManager.kt`, `session/TerminalSession.kt` | Open sessions keyed by session UUID; session owns the emulator |
| Terminal | `terminal/` (vendored Termux, Apache-2.0), `session/SshTerminal.kt`, `ui/TerminalSurface.kt`, `ui/ExtraKeysRow.kt` | Real VT/xterm emulator; SSH bytes drive it through a `Transport` instead of a PTY; the view is the input surface, with an on-screen ESC/CTRL/ALT/TAB/arrow row |
| Tests | `app/src/test/` | Validation, format refusal, ssh args, config reader, vault roundtrip + Swift-generated golden fixture |

## Invariants — do not break these

1. **Master password on every launch.** `prepareForLaunch()` always locks; password input is disabled while locked. There is no "remember me".
2. **One active vault file.** `~/Library/Application Support/Maine/passwords.maine` holds hosts, credentials and session snapshots together. `hosts.json` is a legacy plaintext leftover — never read it as live storage, and tell users to archive/remove it after migration.
3. **Sessions are keyed by session UUID, not host UUID.** Restore reconnects automatically after unlock, with persisted history kept as scrollback (never replayed as commands).
4. **Close vs quit.** Explicit close deletes the encrypted session record first (failure keeps the session visible). Quit only stops processes and keeps records.
5. **Terminals stay mounted.** Switching sessions must never reparent a terminal NSView. Layout is bounded by the window viewport — terminal fitting size or banners must not grow the split view (this was a real shipped regression: exit-255 overflow).
6. **Sidebar-only navigation.** No tab bar, no window title/toolbar, no All Hosts page. Entire sidebar rows are clickable; hover and selection share identical boundaries.
7. **Ad-hoc signing only.** Provisioned signing / Touch ID was implemented, then deliberately rolled back (weekly re-signing burden). Do not reintroduce without asking. Never invent entitlements or pick a signing certificate silently.
8. **Diagnostics are metadata-only and opt-in.** Never log passwords, keystrokes, hostnames, usernames, custom session names, terminal contents or screenshots. Debug regions (`.debugRegion`) record geometry only.
9. **Motion respects the OS.** All transitions/press feedback check Reduce Motion.

## Working agreement

- **Reproduce before fixing.** Write the failing regression test first (see `WorkspaceLifecycleTests.swift` for the pattern), then fix, then run the full suite.
- **Verify by execution.** Run `swift test` after every behavior change; rebuild the release app when the user will retest it.
- **Docs travel with behavior.** Update `README.md` and `docs/architecture.md` (plus `docs/macos-design.md` for layout changes) in the same change.
- **Icons are scripted.** `scripts/render-mark.swift` generates the active M monogram; `render-icon.swift` (prompt mark) is kept for future use. Rebuild `.icns` with `sips` + `iconutil` per the script headers; never commit ad-hoc binaries outside `Resources/`.
- **Git hygiene.** `build/` and `.build/` are ignored — never force-add them. Commit only when asked; never push unless asked.
- **Ask before expanding scope.** Pre-written commands, sync, key management and new platforms are explicitly on hold / not planned.
