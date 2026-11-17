# Linux terminal spike — result: GO

Date: 2026-11-17. Spec: `../specs/2026-11-17-linux-client-design.md` (Phases → 0).

The spike was promoted instead of thrown away: it became `apps/linux/crates/maine-term`
(headless PTY + `alacritty_terminal`, no Qt) and `apps/linux/crates/maine-app`
(cxx-qt app with a C++ `TerminalItem` over a small C ABI in `src/ffi.rs`).
cxx-qt 0.10.0 builds against Qt 6.11.2 on this machine.

| Criterion | Result | Evidence |
| --- | --- | --- |
| cxx-qt 0.10 builds on Qt 6.11 | pass | `cargo build -p maine-app` |
| vim renders correctly | pass | offscreen screenshot of `vim /etc/os-release` |
| htop renders correctly | pass | offscreen screenshot of `htop` (colors, meters, function bar) |
| tmux | not run | tmux is not installed here |
| Large output stays responsive | pass | 102 MB through the PTY with a 60 fps snapshot loop: 0.73 s, ~139 MB/s (release) — `cargo test -p maine-term --release throughput -- --ignored --nocapture` |
| Keyboard: modifiers, arrows, F-keys, Alt, Ctrl | pass | real `QKeyEvent`s through `keyPressEvent` into `cat -v`: `a ^A ^[[A ^[[1;5D Ж é ^[x ^[[15~ ^? ^C` |
| Resize reflows and sends SIGWINCH | pass | `maine-term` test `child_sees_the_pty_size_and_resize_sends_sigwinch` |
| Selection and copy | pass (engine) | `scrollback_and_selection_copy`; the Qt mouse handlers are written but only exercised on a real desktop |
| Cyrillic typing and paste | pass | synthetic `Ж` key event; UTF-8 passthrough tests |
| Wide characters and emoji keep columns aligned | pass | screenshot: `日本語`, `ЖЁЯ`, `😀`, box drawing |
| Compose / IME through Wayland text-input | **not verified** | `inputMethodEvent` commits text; needs a real-desktop check |

Known gaps, none blocking: box-drawing and block glyphs come from the font and show
hairline gaps (draw them procedurally later); cursor does not blink; no IME preedit
display; tmux not exercised.

Manual desktop check (opens a real window, so it is not part of any test run):

```sh
cd apps/linux && cargo run -p maine-app
```

Decision: go. Continue to phase 2 on this base.
