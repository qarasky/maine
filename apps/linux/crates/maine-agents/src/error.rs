use std::fmt;

/// A failure in the agent plumbing (socket, token store, persistence). The
/// message is user-facing; it never contains tokens, passwords or output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentError(pub String);

impl AgentError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AgentError {}
