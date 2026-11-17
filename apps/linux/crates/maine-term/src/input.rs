use crate::snapshot::Modes;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Modifiers {
    /// xterm modifier parameter: 1 + shift(1) + alt(2) + ctrl(4) + meta(8); 1 = none.
    fn param(self) -> u8 {
        1 + self.shift as u8 + 2 * self.alt as u8 + 4 * self.ctrl as u8 + 8 * self.meta as u8
    }

    fn any(self) -> bool {
        self.param() != 1
    }
}

/// Keys the UI layer maps from the toolkit; everything printable arrives as `text`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Text,
    Enter,
    Tab,
    Backtab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    F(u8),
}

#[derive(Debug, Clone)]
pub struct KeyEvent<'a> {
    pub key: Key,
    pub text: &'a str,
    pub mods: Modifiers,
}

/// Bytes to write to the PTY for a key press, xterm style.
pub fn encode_key(event: &KeyEvent, modes: &Modes) -> Vec<u8> {
    let m = event.mods;
    let csi_mod = |final_byte: char, base: &str| -> Vec<u8> {
        if m.any() {
            format!("\x1b[1;{}{}", m.param(), final_byte).into_bytes()
        } else if base.is_empty() {
            format!("\x1b[{final_byte}").into_bytes()
        } else {
            format!("\x1b{base}{final_byte}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if m.any() {
            format!("\x1b[{n};{}~", m.param()).into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    let arrow = |c: char| csi_mod(c, if modes.app_cursor { "O" } else { "[" });
    let out = match event.key {
        Key::Up => arrow('A'),
        Key::Down => arrow('B'),
        Key::Right => arrow('C'),
        Key::Left => arrow('D'),
        Key::Home => csi_mod('H', if modes.app_cursor { "O" } else { "[" }),
        Key::End => csi_mod('F', if modes.app_cursor { "O" } else { "[" }),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::Enter => b"\r".to_vec(),
        Key::Tab => b"\t".to_vec(),
        Key::Backtab => b"\x1b[Z".to_vec(),
        Key::Backspace => {
            if m.ctrl {
                vec![0x08]
            } else {
                vec![0x7f]
            }
        }
        Key::Escape => vec![0x1b],
        Key::F(n) => match n {
            1..=4 => {
                let c = (b'P' + n - 1) as char;
                if m.any() {
                    format!("\x1b[1;{}{c}", m.param()).into_bytes()
                } else {
                    format!("\x1bO{c}").into_bytes()
                }
            }
            5 => tilde(15),
            6 => tilde(17),
            7 => tilde(18),
            8 => tilde(19),
            9 => tilde(20),
            10 => tilde(21),
            11 => tilde(23),
            12 => tilde(24),
            _ => Vec::new(),
        },
        Key::Text => return encode_text(event),
    };
    // Alt prefixes ESC for keys that have no modifier parameter form.
    if m.alt
        && matches!(
            event.key,
            Key::Enter | Key::Tab | Key::Backspace | Key::Escape
        )
    {
        let mut v = vec![0x1b];
        v.extend(out);
        return v;
    }
    out
}

fn encode_text(event: &KeyEvent) -> Vec<u8> {
    let m = event.mods;
    let mut chars = event.text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        // Multi-char text (IME commit, dead-key compose): send as typed.
        let mut v = Vec::new();
        if m.alt && !event.text.is_empty() {
            v.push(0x1b);
        }
        v.extend(event.text.as_bytes());
        return v;
    };
    if m.ctrl {
        let byte = match c {
            'a'..='z' => Some(c as u8 - b'a' + 1),
            'A'..='Z' => Some(c as u8 - b'A' + 1),
            ' ' | '@' | '2' => Some(0),
            '[' | '3' => Some(0x1b),
            '\\' | '4' => Some(0x1c),
            ']' | '5' => Some(0x1d),
            '^' | '6' => Some(0x1e),
            '_' | '7' | '/' => Some(0x1f),
            '8' => Some(0x7f),
            _ => None,
        };
        if let Some(b) = byte {
            return if m.alt { vec![0x1b, b] } else { vec![b] };
        }
    }
    let mut buf = [0u8; 4];
    let bytes = c.encode_utf8(&mut buf).as_bytes();
    let mut v = Vec::new();
    if m.alt {
        v.push(0x1b);
    }
    v.extend(bytes);
    v
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    Press,
    Release,
    Motion,
}

#[derive(Debug, Clone, Copy)]
pub struct MouseEvent {
    pub button: MouseButton,
    pub kind: MouseKind,
    /// Zero-based cell position.
    pub col: usize,
    pub row: usize,
    pub mods: Modifiers,
}

/// xterm mouse report for the active mouse mode, or `None` if this event is not
/// reported in that mode.
pub fn encode_mouse(event: &MouseEvent, modes: &Modes) -> Option<Vec<u8>> {
    if !modes.mouse_reporting() {
        return None;
    }
    if event.kind == MouseKind::Motion {
        let dragging = event.button != MouseButton::None;
        if !(modes.mouse_motion || (modes.mouse_drag && dragging)) {
            return None;
        }
    }
    let base = match event.button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::None => 3,
        MouseButton::WheelUp => 64,
        MouseButton::WheelDown => 65,
    };
    let mut code = base;
    if event.mods.shift {
        code += 4;
    }
    if event.mods.alt {
        code += 8;
    }
    if event.mods.ctrl {
        code += 16;
    }
    if event.kind == MouseKind::Motion {
        code += 32;
    }
    let (x, y) = (event.col + 1, event.row + 1);
    if modes.sgr_mouse {
        let suffix = if event.kind == MouseKind::Release {
            'm'
        } else {
            'M'
        };
        return Some(format!("\x1b[<{code};{x};{y}{suffix}").into_bytes());
    }
    // Legacy X10 encoding: release is button 3; coordinates are limited to 223.
    if x > 223 || y > 223 {
        return None;
    }
    let code = if event.kind == MouseKind::Release {
        3 + (code & !3)
    } else {
        code
    };
    Some(vec![
        0x1b,
        b'[',
        b'M',
        32 + code as u8,
        32 + x as u8,
        32 + y as u8,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: Key, text: &str, mods: Modifiers) -> Vec<u8> {
        encode_key(&KeyEvent { key, text, mods }, &Modes::default())
    }

    const NONE: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
        alt: false,
        meta: false,
    };

    #[test]
    fn plain_text_and_utf8() {
        assert_eq!(key(Key::Text, "a", NONE), b"a");
        assert_eq!(key(Key::Text, "ж", NONE), "ж".as_bytes());
        assert_eq!(key(Key::Text, "é", NONE), "é".as_bytes());
    }

    #[test]
    fn ctrl_letters_and_alt_prefix() {
        let ctrl = Modifiers { ctrl: true, ..NONE };
        let alt = Modifiers { alt: true, ..NONE };
        assert_eq!(key(Key::Text, "c", ctrl), [3]);
        assert_eq!(key(Key::Text, "C", ctrl), [3]);
        assert_eq!(key(Key::Text, "[", ctrl), [0x1b]);
        assert_eq!(key(Key::Text, "x", alt), b"\x1bx");
        assert_eq!(key(Key::Backspace, "", NONE), [0x7f]);
    }

    #[test]
    fn arrows_follow_application_cursor_mode() {
        let normal = Modes::default();
        let app = Modes {
            app_cursor: true,
            ..normal
        };
        let ev = KeyEvent {
            key: Key::Up,
            text: "",
            mods: NONE,
        };
        assert_eq!(encode_key(&ev, &normal), b"\x1b[A");
        assert_eq!(encode_key(&ev, &app), b"\x1bOA");
        let shift = KeyEvent {
            key: Key::Up,
            text: "",
            mods: Modifiers {
                shift: true,
                ..NONE
            },
        };
        assert_eq!(encode_key(&shift, &app), b"\x1b[1;2A");
    }

    #[test]
    fn navigation_and_function_keys() {
        assert_eq!(key(Key::PageUp, "", NONE), b"\x1b[5~");
        assert_eq!(
            key(Key::Delete, "", Modifiers { ctrl: true, ..NONE }),
            b"\x1b[3;5~"
        );
        assert_eq!(key(Key::F(1), "", NONE), b"\x1bOP");
        assert_eq!(key(Key::F(5), "", NONE), b"\x1b[15~");
        assert_eq!(key(Key::F(12), "", NONE), b"\x1b[24~");
        assert_eq!(key(Key::Backtab, "", NONE), b"\x1b[Z");
    }

    #[test]
    fn mouse_reports_follow_the_mode() {
        let ev = MouseEvent {
            button: MouseButton::Left,
            kind: MouseKind::Press,
            col: 4,
            row: 2,
            mods: NONE,
        };
        assert_eq!(encode_mouse(&ev, &Modes::default()), None);
        let sgr = Modes {
            mouse_click: true,
            sgr_mouse: true,
            ..Modes::default()
        };
        assert_eq!(encode_mouse(&ev, &sgr).unwrap(), b"\x1b[<0;5;3M");
        let rel = MouseEvent {
            kind: MouseKind::Release,
            ..ev
        };
        assert_eq!(encode_mouse(&rel, &sgr).unwrap(), b"\x1b[<0;5;3m");
        let legacy = Modes {
            mouse_click: true,
            ..Modes::default()
        };
        assert_eq!(
            encode_mouse(&ev, &legacy).unwrap(),
            [0x1b, b'[', b'M', 32, 37, 35]
        );
        let motion = MouseEvent {
            kind: MouseKind::Motion,
            button: MouseButton::None,
            ..ev
        };
        assert_eq!(encode_mouse(&motion, &sgr), None);
    }
}
