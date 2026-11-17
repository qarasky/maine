//! MCP agent access for the Maine Linux client, UI-free. Mirrors the macOS
//! `Agents/` layer: the output buffer agents read, command completion (Maine's
//! marker, OSC 133, quiet period), the per-session driver, the access rules and
//! activity log shapes, paired client tokens, the length-prefixed socket
//! protocol, the Unix-socket server and tool handler, and the `maine --mcp`
//! stdio bridge. The app supplies sessions and persistence through
//! [`AgentSession`], [`AgentHost`] and [`SecretStore`].
//!
//! Invariants (AGENTS.md 8 and 12): diagnostics are metadata-only and nothing
//! here logs commands, keys, output, screen text, host names, tokens or
//! passwords; a saved password is typed into a detected prompt and never
//! returned; a host or session an agent may not use answers "not found".
#![forbid(unsafe_code)]

pub mod access;
pub mod adapter;
pub mod apple;
pub mod audit;
pub mod bridge;
pub mod catalog;
pub mod client;
pub mod clients;
pub mod clock;
pub mod completion;
pub mod coordinator;
pub mod driver;
pub mod error;
pub mod host;
pub mod json;
pub mod output;
pub mod protocol;
pub mod server;
pub mod session;
mod sock;
pub mod tools;

pub use access::AgentAccess;
pub use adapter::TermAgentSession;
pub use apple::AppleDate;
pub use audit::{AgentAuditEntry, AuditKind};
pub use bridge::{run_bridge, run_bridge_from_env, serve_lines, McpBridgeSession};
pub use client::{AgentSocketClient, ClientError};
pub use clients::{AgentClient, AgentClientStore, MemorySecretStore, Pairing, SecretStore};
pub use clock::{Clock, ManualClock, SystemClock};
pub use coordinator::AgentCoordinator;
pub use driver::{AgentDriver, AgentKey, DriverError, ReadResult, RunResult, ScreenResult};
pub use error::AgentError;
pub use host::{AgentHost, AgentSessionRef, HandOffError, CONNECTING_LABEL};
pub use output::{AgentOutput, MASK_REPLACEMENT};
pub use server::{default_socket_path, AgentServer, NOT_PAIRED};
pub use session::{AgentSession, OutputTap, ScreenText};
pub use tools::{AgentCallHandler, AgentToolHandler};
