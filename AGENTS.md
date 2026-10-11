# AGENTS.md

Instructions for AI coding agents working in this repository. Humans use `README.md`; this file is the contributor shortcut.

## What this is

Maine is a local-first SSH client for macOS (Swift 6, SwiftUI, macOS 14+). Single Swift package at `apps/macos` with one dependency (SwiftTerm). No backend, no sync, no telemetry. Shared cross-client contract lives in `packages/schema`, not shared code.

```
maine/
├── apps/macos/     The app: Sources/Maine, Tests/MaineTests, scripts/, Resources/
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
