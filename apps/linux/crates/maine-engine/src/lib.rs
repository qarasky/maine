//! Application core of the Maine Linux client, with no UI toolkit: the unlocked
//! vault and host store, terminal sessions keyed by session UUID, the askpass
//! broker and the lock/quit lifecycle. The Qt layer is a thin shell over this.
#![forbid(unsafe_code)]

pub mod askpass;
pub mod diag;
mod engine;
mod paths;
mod sessions;
pub mod view;

pub use engine::{Engine, EngineState};
pub use paths::Paths;
pub use sessions::{AgentLink, Launcher, SessionInfo, SessionManager, SessionState, SshLauncher};
