//! What the driver needs from a terminal session. The app implements this for
//! its sessions (an adapter for `maine_term::TermSession` is in
//! [`crate::adapter`]); tests use scripted fakes.

use crate::completion::looks_like_shell_prompt;
use std::time::Instant;

/// The visible screen as text, and where the cursor is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScreenText {
    pub rows: Vec<String>,
    pub cursor_row: usize,
    pub cursor_col: usize,
}

impl ScreenText {
    /// The last row with anything on it.
    pub fn last_non_empty_row(&self) -> &str {
        self.rows
            .iter()
            .rev()
            .find(|row| !row.trim().is_empty())
            .map_or("", String::as_str)
    }
}

/// Receives every raw byte the child writes, before the emulator sees it.
pub type OutputTap = Box<dyn Fn(&[u8]) + Send + Sync>;

pub trait AgentSession: Send + Sync {
    /// Writes bytes to the child's stdin without counting as the user typing.
    fn send_input(&self, bytes: &[u8]);

    /// Installs (or with `None` removes) the raw-output tap. A session has one
    /// tap; installing a driver's replaces any other.
    fn set_output_tap(&self, tap: Option<OutputTap>);

    /// The visible screen as text.
    fn screen_text(&self) -> ScreenText;

    /// The child is still running.
    fn is_running(&self) -> bool;

    /// When the user last typed into this session, if ever. The driver waits
    /// while this is recent: human input wins. Sessions that cannot tell return `None`.
    fn last_user_input(&self) -> Option<Instant> {
        None
    }

    /// Told when an agent starts or stops driving the session. While driven,
    /// OSC 52 clipboard access, link clicks and the bell must stay off.
    fn set_agent_driven(&self, _driven: bool) {}

    /// The last non-empty screen line looks like a shell prompt. A full-screen
    /// program or a running command almost never ends that way.
    fn is_at_shell_prompt(&self) -> bool {
        looks_like_shell_prompt(self.screen_text().last_non_empty_row())
    }
}
