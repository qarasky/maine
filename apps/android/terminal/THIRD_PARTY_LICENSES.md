The `termux/termux-app` repository is released under [GPLv3 only](https://www.gnu.org/licenses/gpl-3.0.html) license.

### Exceptions

- [Terminal Emulator for Android](https://github.com/jackpal/Android-Terminal-Emulator) code is used which is released under [Apache 2.0](https://www.apache.org/licenses/LICENSE-2.0) license. Check [`terminal-view`](terminal-view) and [`terminal-emulator`](terminal-emulator) libraries.
- Check [`termux-shared/LICENSE.md`](termux-shared/LICENSE.md) for `termux-shared` library related exceptions.

---

## Vendored code in this module

This module contains source copied from [termux/termux-app](https://github.com/termux/termux-app),
specifically the `terminal-emulator` and `terminal-view` libraries. Those two
libraries are released under the **Apache License 2.0** (stated under
"Exceptions" in the upstream license above), not the GPLv3 that covers the rest
of termux-app.

- `src/main/java/com/termux/terminal/**` — from `terminal-emulator`
- `src/main/java/com/termux/view/**` — from `terminal-view`
- `src/main/res/**` — from `terminal-view`

Local change: `TerminalSession.java` gains a public `Transport` interface and a
transport-backed constructor/`initializeTransport()` path so an SSH stream can
drive the emulator instead of a native PTY. The PTY path is otherwise untouched.
