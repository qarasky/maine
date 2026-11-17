//! A headless terminal session: a child process on a PTY, an
//! `alacritty_terminal` emulator for screen state, raw-byte taps for agents,
//! cell snapshots for painting, and key/mouse encoding. No UI toolkit.
#![forbid(unsafe_code)]

mod colors;
mod input;
mod session;
mod snapshot;

pub use colors::Palette;
pub use input::{
    encode_key, encode_mouse, Key, KeyEvent, Modifiers, MouseButton, MouseEvent, MouseKind,
};
pub use session::{SessionConfig, SessionEvent, TermSession};
pub use snapshot::{Cell, CellFlags, CursorShape, Modes, Snapshot};
