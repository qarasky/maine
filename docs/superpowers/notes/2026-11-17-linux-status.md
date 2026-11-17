# Linux client status (2026-11-17)

Built in one session from the design in `../specs/2026-11-17-linux-client-design.md`.
Phases 0-4 exist; nothing is committed. This note says what was verified, how, and what was not.

## What is verified

| Area | How |
| --- | --- |
| Vault v2 interop with macOS | Swift-generated golden file opens in Rust (`maine-core/tests/golden.rs`) |
| Core, terminal, engine, sync, agents | `cargo test --workspace`: unit and integration tests (over 400 at the time of writing); sync against the real `maine-sync-server` binary; agents against real shell sessions |
| Askpass | the real `maine` binary in helper mode fetched a saved password over the socket |
| Sync in the app | instance A created a slot, instance B joined with the token and showed A's hosts and groups; Secret Service round trip tested against the real KWallet (ignored tests, run by hand) |
| MCP in the app | real `maine --mcp` bridge against the running app: initialize, tools/list, list_hosts (scoped), open_session, run (output and exit code), read_screen, close |
| UI | offscreen screenshots of the gate, sidebar, groups, nested sessions, header, host editor, ssh prompt dialog, Settings (Sync, Agents) |
| Terminal rendering | vim, htop, colors, bold/italic/underline, truecolor, CJK, Cyrillic, emoji; real `QKeyEvent`s mapped to the right bytes |

Tests never open a window on the desktop: UI checks use `QT_QPA_PLATFORM=offscreen`.

## Verified on the real desktop (KDE Plasma 6, Wayland, 2026-11-17)

Driven with real keyboard and mouse events (ydotool, kdotool, spectacle) against the debug build,
default `org.kde.desktop` style, isolated data directory.

| Area | Result |
| --- | --- |
| Plasma look | Breeze dark, native KWin title bar, correct dialogs |
| Gate, Ctrl+N editor, Tab order, Save | works |
| Terminal | typing, colors, Cyrillic paste (Ctrl+Shift+V), vim (alt screen, status line), Shift+PageUp scrollback |
| Mouse | drag selection highlight, middle-click paste of the selection |
| Drag and drop | dragging a host onto a group header moves it; survives quit and relaunch |
| Resize | window resize reflows the terminal and the PTY size follows (`stty size` 37x98 to 26x67) |
| Tray | icon registered with the StatusNotifierWatcher; menu Show / Lock vault / Quit work |
| Close to tray | closing hides the window and the process keeps running |
| Single instance | a second launch exits and raises the first window |
| Lock / unlock | tray lock shows the gate; unlock with the keyboard; sessions stay live |
| Quit and restore | after quit and relaunch the sessions, names, scrollback, groups and order are back |

Bugs this pass found and fixed: tray Quit did nothing (`Qt.quit()` closes windows first and the
hide-to-tray handler vetoed it); the single-instance socket was not started in the debug-shell
engine; the debug shell launcher inherited `TERM=dumb`. A regression test for the alternate screen
was added to `maine-term`.

## Still not verified

* Compose, dead keys and IME preedit (ydotool types US keycodes only).
* Non-US keyboard layouts (Ctrl+letter on a Cyrillic layout in particular).
* tmux (not installed on the build machine).
* Maine's own icon in the tray and taskbar (needs `scripts/install.sh` so the `.desktop` file and icon exist; the tray shows a stock terminal icon until then).
* A real `ssh` connection with a saved password through askpass (no reachable sshd here; askpass was verified with the real binary against the socket).

## Known gaps

* Box-drawing and block glyphs come from the font and show hairline gaps; draw them procedurally.
* The cursor does not blink; no IME preedit display.
* `Reduce Motion` is not wired (the UI has almost no animation).
* There is no change-master-password flow on Linux.
* `ureq` uses bundled webpki roots (not the system store) and never follows redirects (sync).
* The agent adapter reports cursor (0,0) for full-screen programs (hidden cursor).
* Sessions restore as scrollback text only, like macOS.

## Process notes

Phase 0 was not thrown away: the spike became `maine-term` and `maine-app`. The phase 2 plan
was not written separately; the code followed the spec directly. Phases 3 and 4 were built by
two delegated engineers (`maine-sync`, `maine-agents`) from briefs, then re-run and integrated
here; their reports and the differences from macOS are in `docs/sync-client.md` and `docs/mcp.md`
("Status on Linux").
