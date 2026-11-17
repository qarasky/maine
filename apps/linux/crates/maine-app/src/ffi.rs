//! C ABI between the C++ `TerminalItem` and `maine-term`. Every pointer
//! argument is owned by the caller unless stated; `MaineTerm` is created by
//! `maine_term_attach` and destroyed by `maine_term_free`.
#![allow(clippy::missing_safety_doc)]

use maine_term::{
    encode_key, encode_mouse, Key, KeyEvent, Modes, Modifiers, MouseButton, MouseEvent, MouseKind,
    SessionEvent, TermSession,
};
use std::ffi::{c_char, c_int, c_void, CStr, CString};

pub struct MaineTerm {
    session: TermSession,
    /// Shared with the agent driver: tracks the human's typing and whether an agent drives the session.
    adapter: std::sync::Arc<maine_agents::TermAgentSession>,
}

/// Cell as the painter sees it.
#[repr(C)]
pub struct MaineCell {
    pub ch: u32,
    pub fg: u32,
    pub bg: u32,
    pub flags: u16,
    pub pad: u16,
}

#[repr(C)]
pub struct MaineSnapshotInfo {
    pub cols: u32,
    pub rows: u32,
    pub cursor_col: i32,
    pub cursor_row: i32,
    /// 0 block, 1 underline, 2 beam, 3 hollow block
    pub cursor_shape: u32,
    pub display_offset: u32,
    pub history: u32,
    pub modes: u32,
    pub default_fg: u32,
    pub default_bg: u32,
}

struct UpdateHook {
    callback: extern "C" fn(*mut c_void),
    user: *mut c_void,
    release: extern "C" fn(*mut c_void),
}

// SAFETY: the C++ side guarantees `callback` and `release` are callable from any thread.
unsafe impl Send for UpdateHook {}
unsafe impl Sync for UpdateHook {}

impl Drop for UpdateHook {
    fn drop(&mut self) {
        (self.release)(self.user);
    }
}

fn modes_bits(m: &Modes) -> u32 {
    (m.app_cursor as u32)
        | (m.bracketed_paste as u32) << 1
        | (m.alt_screen as u32) << 2
        | (m.focus_reporting as u32) << 3
        | (m.mouse_click as u32) << 4
        | (m.mouse_drag as u32) << 5
        | (m.mouse_motion as u32) << 6
        | (m.sgr_mouse as u32) << 7
        | (m.alternate_scroll as u32) << 8
        | (m.app_keypad as u32) << 9
}

fn mods(bits: u8) -> Modifiers {
    Modifiers {
        shift: bits & 1 != 0,
        ctrl: bits & 2 != 0,
        alt: bits & 4 != 0,
        meta: bits & 8 != 0,
    }
}

/// Attaches a view to the engine's session with this UUID. Returns null when
/// there is no such session (or it never started). The view only borrows the
/// session: freeing the handle detaches it and never stops the process.
#[no_mangle]
pub unsafe extern "C" fn maine_term_attach(
    session_id: *const c_char,
    callback: extern "C" fn(*mut c_void),
    user: *mut c_void,
    release: extern "C" fn(*mut c_void),
) -> *mut MaineTerm {
    // Created first so `user` is released on every path, including failure.
    let hook = std::sync::Arc::new(UpdateHook {
        callback,
        user,
        release,
    });
    let Ok(id) = uuid::Uuid::parse_str(&CStr::from_ptr(session_id).to_string_lossy()) else {
        return std::ptr::null_mut();
    };
    let found = crate::state::with_engine(|e| {
        let term = e.sessions().terminal(id);
        e.sessions().mark_sized(id);
        term
    });
    match found {
        Some(session) => {
            session.set_update_callback(move || (hook.callback)(hook.user));
            let adapter = crate::agents::adapter_for(id, &session);
            Box::into_raw(Box::new(MaineTerm { session, adapter }))
        }
        None => std::ptr::null_mut(),
    }
}

/// The human typed, pasted, clicked or composed text: an agent must wait (human input wins).
#[no_mangle]
pub unsafe extern "C" fn maine_term_note_input(term: *mut MaineTerm) {
    (*term).adapter.note_user_input();
}

/// Whether an agent currently drives the session (OSC 52, link clicks and the bell stay off then).
#[no_mangle]
pub unsafe extern "C" fn maine_term_agent_driven(term: *mut MaineTerm) -> bool {
    (*term).adapter.is_agent_driven()
}

/// Detaches the view. The session keeps running in the engine.
#[no_mangle]
pub unsafe extern "C" fn maine_term_free(term: *mut MaineTerm) {
    if term.is_null() {
        return;
    }
    let term = Box::from_raw(term);
    term.session.set_update_callback(|| {});
}

fn t<'a>(term: *mut MaineTerm) -> &'a TermSession {
    // SAFETY: callers pass a live handle from `maine_term_spawn`.
    unsafe { &(*term).session }
}

#[no_mangle]
pub unsafe extern "C" fn maine_term_resize(term: *mut MaineTerm, cols: u16, rows: u16) {
    t(term).resize(cols, rows);
}

#[no_mangle]
pub unsafe extern "C" fn maine_term_send(term: *mut MaineTerm, bytes: *const u8, len: usize) {
    t(term).send(std::slice::from_raw_parts(bytes, len).to_vec());
}

#[no_mangle]
pub unsafe extern "C" fn maine_term_paste(term: *mut MaineTerm, text: *const c_char) {
    t(term).paste(&CStr::from_ptr(text).to_string_lossy());
}

/// `key`: 0 text, 1 Enter, 2 Tab, 3 Backtab, 4 Backspace, 5 Esc, 6 Up, 7 Down, 8 Left, 9 Right,
/// 10 Home, 11 End, 12 PageUp, 13 PageDown, 14 Insert, 15 Delete, 100+n = Fn. `mods`: 1 shift, 2 ctrl, 4 alt, 8 meta.
#[no_mangle]
pub unsafe extern "C" fn maine_term_key(
    term: *mut MaineTerm,
    key: c_int,
    text: *const c_char,
    mod_bits: u8,
) {
    let key = match key {
        1 => Key::Enter,
        2 => Key::Tab,
        3 => Key::Backtab,
        4 => Key::Backspace,
        5 => Key::Escape,
        6 => Key::Up,
        7 => Key::Down,
        8 => Key::Left,
        9 => Key::Right,
        10 => Key::Home,
        11 => Key::End,
        12 => Key::PageUp,
        13 => Key::PageDown,
        14 => Key::Insert,
        15 => Key::Delete,
        n if (101..=124).contains(&n) => Key::F((n - 100) as u8),
        _ => Key::Text,
    };
    let text = if text.is_null() {
        String::new()
    } else {
        CStr::from_ptr(text).to_string_lossy().into_owned()
    };
    let session = t(term);
    session.scroll_to_bottom();
    let bytes = encode_key(
        &KeyEvent {
            key,
            text: &text,
            mods: mods(mod_bits),
        },
        &session.modes(),
    );
    if !bytes.is_empty() {
        session.send(bytes);
    }
}

/// `button`: 0 left, 1 middle, 2 right, 3 wheel up, 4 wheel down, 5 none. `kind`: 0 press, 1 release, 2 motion.
/// Returns true when the event was reported to the program (and must not start a selection).
#[no_mangle]
pub unsafe extern "C" fn maine_term_mouse(
    term: *mut MaineTerm,
    button: c_int,
    kind: c_int,
    col: u32,
    row: u32,
    mod_bits: u8,
) -> bool {
    let button = match button {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        3 => MouseButton::WheelUp,
        4 => MouseButton::WheelDown,
        _ => MouseButton::None,
    };
    let kind = match kind {
        0 => MouseKind::Press,
        1 => MouseKind::Release,
        _ => MouseKind::Motion,
    };
    let session = t(term);
    let event = MouseEvent {
        button,
        kind,
        col: col as usize,
        row: row as usize,
        mods: mods(mod_bits),
    };
    match encode_mouse(&event, &session.modes()) {
        Some(bytes) => {
            session.send(bytes);
            true
        }
        None => false,
    }
}

/// Copies the visible screen into `out` (`cap` cells, row-major) and fills `info`.
/// Returns the number of cells written (0 if `cap` is too small for the current size).
#[no_mangle]
pub unsafe extern "C" fn maine_term_snapshot(
    term: *mut MaineTerm,
    out: *mut MaineCell,
    cap: usize,
    info: *mut MaineSnapshotInfo,
) -> usize {
    let snap = t(term).snapshot();
    let (cursor_col, cursor_row, shape) = match snap.cursor {
        Some((c, r, s)) => (c as i32, r as i32, s),
        None => (-1, -1, maine_term::CursorShape::Block),
    };
    *info = MaineSnapshotInfo {
        cols: snap.cols as u32,
        rows: snap.rows as u32,
        cursor_col,
        cursor_row,
        cursor_shape: match shape {
            maine_term::CursorShape::Block => 0,
            maine_term::CursorShape::Underline => 1,
            maine_term::CursorShape::Beam => 2,
            _ => 3,
        },
        display_offset: snap.display_offset as u32,
        history: snap.history as u32,
        modes: modes_bits(&snap.modes),
        default_fg: snap.default_fg,
        default_bg: snap.default_bg,
    };
    if snap.cells.len() > cap {
        return 0;
    }
    for (i, c) in snap.cells.iter().enumerate() {
        *out.add(i) = MaineCell {
            ch: c.ch as u32,
            fg: c.fg,
            bg: c.bg,
            flags: c.flags,
            pad: 0,
        };
    }
    snap.cells.len()
}

#[no_mangle]
pub unsafe extern "C" fn maine_term_scroll(term: *mut MaineTerm, lines: c_int) {
    t(term).scroll(lines);
}

#[no_mangle]
pub unsafe extern "C" fn maine_term_selection_start(
    term: *mut MaineTerm,
    col: u32,
    row: u32,
    right: bool,
    block: bool,
) {
    t(term).selection_start(col as usize, row as usize, right, block);
}

#[no_mangle]
pub unsafe extern "C" fn maine_term_selection_update(
    term: *mut MaineTerm,
    col: u32,
    row: u32,
    right: bool,
) {
    t(term).selection_update(col as usize, row as usize, right);
}

#[no_mangle]
pub unsafe extern "C" fn maine_term_selection_word(term: *mut MaineTerm, col: u32, row: u32) {
    t(term).selection_word(col as usize, row as usize);
}

#[no_mangle]
pub unsafe extern "C" fn maine_term_selection_line(term: *mut MaineTerm, col: u32, row: u32) {
    t(term).selection_line(col as usize, row as usize);
}

#[no_mangle]
pub unsafe extern "C" fn maine_term_selection_clear(term: *mut MaineTerm) {
    t(term).selection_clear();
}

/// Heap string the caller frees with `maine_string_free`, or null when nothing is selected.
#[no_mangle]
pub unsafe extern "C" fn maine_term_selection_text(term: *mut MaineTerm) -> *mut c_char {
    match t(term).selection_text().and_then(|s| CString::new(s).ok()) {
        Some(s) => s.into_raw(),
        None => std::ptr::null_mut(),
    }
}

/// Visible screen as text (for tests and accessibility); free with `maine_string_free`.
#[no_mangle]
pub unsafe extern "C" fn maine_term_screen_text(term: *mut MaineTerm) -> *mut c_char {
    CString::new(t(term).snapshot().text().replace('\0', ""))
        .unwrap_or_default()
        .into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn maine_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

/// Next pending event: `kind` 1 title (text), 2 bell, 3 clipboard store (text), 4 exited (`code`, -1 unknown).
/// Returns false when the queue is empty.
#[no_mangle]
pub unsafe extern "C" fn maine_term_next_event(
    term: *mut MaineTerm,
    kind: *mut c_int,
    text: *mut *mut c_char,
    code: *mut c_int,
) -> bool {
    thread_local! { static PENDING: std::cell::RefCell<std::collections::VecDeque<SessionEvent>> = const { std::cell::RefCell::new(std::collections::VecDeque::new()) }; }
    PENDING.with(|p| {
        let mut p = p.borrow_mut();
        p.extend(t(term).take_events());
        let Some(event) = p.pop_front() else {
            return false;
        };
        *text = std::ptr::null_mut();
        *code = 0;
        match event {
            SessionEvent::Title(s) => {
                *kind = 1;
                *text = CString::new(s.replace('\0', ""))
                    .unwrap_or_default()
                    .into_raw();
            }
            SessionEvent::Bell => *kind = 2,
            SessionEvent::ClipboardStore(s) => {
                *kind = 3;
                *text = CString::new(s.replace('\0', ""))
                    .unwrap_or_default()
                    .into_raw();
            }
            SessionEvent::Exited(c) => {
                *kind = 4;
                *code = c.unwrap_or(-1);
            }
        }
        true
    })
}

#[no_mangle]
pub unsafe extern "C" fn maine_term_is_running(term: *mut MaineTerm) -> bool {
    t(term).is_running()
}
