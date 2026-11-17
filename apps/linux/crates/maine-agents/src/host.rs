//! What the server and the tools need from the app. The app implements
//! [`AgentHost`]; tests use a fake. Everything the app owns (the host list, the
//! vault, the sealed local state, the sessions and their terminals) stays behind
//! this trait.

use crate::access::AgentAccess;
use crate::audit::AgentAuditEntry;
use crate::clients::AgentClient;
use crate::driver::AgentDriver;
use crate::error::AgentError;
use crate::session::AgentSession;
use maine_core::model::{Host, HostLayout};
use std::fmt;
use std::sync::Arc;
use uuid::Uuid;
use zeroize::Zeroizing;

/// The state label a session shows while its remote shell is not ready.
pub const CONNECTING_LABEL: &str = "Connecting…";

/// Why a session could not be handed to an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandOffError {
    NotFound,
    NotAllowed,
    AlreadyDriven,
    NotAtPrompt,
}

impl fmt::Display for HandOffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotFound => "That session or agent no longer exists.",
            Self::NotAllowed => "Agent access is off, or this host is not allowed. Turn it on in Settings > Agents.",
            Self::AlreadyDriven => "An agent is already driving this session.",
            Self::NotAtPrompt => "Wait until the shell is at a prompt, with no command or full-screen program running.",
        })
    }
}

impl std::error::Error for HandOffError {}

/// A snapshot of one open session, as the app sees it. Fetched fresh for each
/// call, so state labels and notices are current.
#[derive(Clone)]
pub struct AgentSessionRef {
    pub id: Uuid,
    pub host_id: Uuid,
    /// The name the sidebar shows ("Session 2", or what the user or agent named it).
    pub name: String,
    /// A short state label for `list_sessions` ("Connected", "Closed (0)", ...).
    pub state_label: String,
    /// The process is running (connecting or connected).
    pub is_live: bool,
    /// The problem worth reporting when the session is not live: a launch
    /// failure, or ssh exiting with an error.
    pub notice: Option<String>,
    pub terminal: Arc<dyn AgentSession>,
    /// The agent driving this session, if any.
    pub driver: Option<Arc<AgentDriver>>,
}

pub trait AgentHost: Send + Sync {
    /// The vault is unlocked. While it is not, every tool fails.
    fn is_unlocked(&self) -> bool;

    /// The host list and groups, in sidebar order.
    fn layout(&self) -> HostLayout;

    /// One host as it is now (its group may have changed since a rule was made).
    /// The tools call this every few milliseconds while a `run` waits: override
    /// it with a direct lookup instead of cloning the layout.
    fn host(&self, id: Uuid) -> Option<Host> {
        self.layout()
            .hosts()
            .iter()
            .find(|host| host.id == id)
            .cloned()
    }

    /// The device-local access rules.
    fn access(&self) -> AgentAccess;

    /// Persists the rules (already pruned) in the sealed local state.
    fn store_access(&self, access: &AgentAccess) -> Result<(), AgentError>;

    fn has_saved_password(&self, host_id: Uuid) -> bool;

    /// The saved password, for `type_password` only. It is typed into the
    /// terminal and never returned, logged or put in an argument list.
    fn saved_password(&self, host_id: Uuid) -> Option<Zeroizing<String>>;

    /// Opens a new session on `host` driven by `client` (an ephemeral session
    /// the user can watch): starts it, builds an [`AgentDriver`] for it and
    /// keeps both. Fails with a user-facing message when nothing could be started.
    fn open_agent_session(
        &self,
        host: &Host,
        client: &AgentClient,
        name: Option<&str>,
    ) -> Result<AgentSessionRef, String>;

    /// Every open session, agent-driven or not.
    fn sessions(&self) -> Vec<AgentSessionRef>;

    fn session(&self, id: Uuid) -> Option<AgentSessionRef> {
        self.sessions().into_iter().find(|session| session.id == id)
    }

    /// Attaches `driver` to a session the user opened (hand-off). With
    /// `handed_off` the session stays an ordinary saved session while driven
    /// and returns to the user, with no "agent disconnected" leftover, when
    /// control ends.
    fn attach_agent(&self, session_id: Uuid, driver: Arc<AgentDriver>, handed_off: bool);

    /// Agent control of a session ended (the crate has already detached the
    /// driver). The session stays open: drop the app's reference to the driver
    /// and, for an agent-opened session, mark it "Agent disconnected".
    fn end_agent_control(&self, session_id: Uuid);

    /// The same as the user's explicit close.
    fn close_session(&self, session_id: Uuid);

    /// Records what an agent did (the encrypted, device-local activity log).
    fn record_audit(&self, entry: AgentAuditEntry);

    /// Metadata-only diagnostics (tool name, ids, outcome, duration, byte
    /// counts). Never commands, keys, output, host names, tokens or passwords:
    /// nothing of that is ever passed here.
    fn diagnostics(&self, _event: &str, _fields: &[(&str, String)]) {}
}
