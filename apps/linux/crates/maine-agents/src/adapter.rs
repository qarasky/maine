//! `maine_term::TermSession` as an [`AgentSession`].

use crate::session::{AgentSession, OutputTap, ScreenText};
use maine_term::TermSession;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

fn screen_text(session: &TermSession) -> ScreenText {
    let snapshot = session.snapshot();
    let rows = (0..snapshot.rows)
        .map(|row| snapshot.row_text(row))
        .collect();
    // A hidden cursor (full-screen programs) has no reported position; the
    // origin is the honest fallback.
    let (cursor_col, cursor_row) = snapshot.cursor.map_or((0, 0), |(col, row, _)| (col, row));
    ScreenText {
        rows,
        cursor_row,
        cursor_col,
    }
}

fn install_tap(session: &TermSession, tap: Option<OutputTap>) {
    match tap {
        Some(tap) => session.set_output_tap(move |bytes| tap(bytes)),
        None => session.set_output_tap(|_| {}),
    }
}

/// The plain adapter: no human-input tracking, no driven flag.
impl AgentSession for TermSession {
    fn send_input(&self, bytes: &[u8]) {
        self.send(bytes.to_vec());
    }

    fn set_output_tap(&self, tap: Option<OutputTap>) {
        install_tap(self, tap);
    }

    fn screen_text(&self) -> ScreenText {
        screen_text(self)
    }

    fn is_running(&self) -> bool {
        TermSession::is_running(self)
    }
}

/// A `TermSession` plus what the driver and the UI need to share about it: the
/// last time the human typed (call [`note_user_input`](Self::note_user_input)
/// from the key handler) and whether an agent currently drives it (read
/// [`is_agent_driven`](Self::is_agent_driven) before honoring OSC 52 clipboard
/// events, link clicks and the bell).
#[derive(Clone)]
pub struct TermAgentSession {
    term: TermSession,
    last_user_input: Arc<Mutex<Option<Instant>>>,
    driven: Arc<AtomicBool>,
}

impl TermAgentSession {
    pub fn new(term: TermSession) -> Self {
        Self {
            term,
            last_user_input: Arc::new(Mutex::new(None)),
            driven: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn term(&self) -> &TermSession {
        &self.term
    }

    /// Records that the user just typed or pasted into the session.
    pub fn note_user_input(&self) {
        *self
            .last_user_input
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    }

    pub fn is_agent_driven(&self) -> bool {
        self.driven.load(Ordering::Acquire)
    }
}

impl AgentSession for TermAgentSession {
    fn send_input(&self, bytes: &[u8]) {
        self.term.send(bytes.to_vec());
    }

    fn set_output_tap(&self, tap: Option<OutputTap>) {
        install_tap(&self.term, tap);
    }

    fn screen_text(&self) -> ScreenText {
        screen_text(&self.term)
    }

    fn is_running(&self) -> bool {
        self.term.is_running()
    }

    fn last_user_input(&self) -> Option<Instant> {
        *self
            .last_user_input
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn set_agent_driven(&self, driven: bool) {
        self.driven.store(driven, Ordering::Release);
    }
}
