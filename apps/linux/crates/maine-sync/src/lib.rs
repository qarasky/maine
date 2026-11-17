//! Sync client for Maine (Linux): profiles, HTTP transport, the whole-file
//! engine and the coordinator that decides when to talk to the server.
//! Qt-free, no unsafe code. See `docs/sync-client.md` and
//! `docs/sync-protocol.md`; the macOS `Sources/Maine/Sync/` is the reference.
#![forbid(unsafe_code)]

pub mod coordinator;
pub mod engine;
pub mod error;
pub mod profile;
pub mod runner;
pub mod testing;
pub mod time;
pub mod token;
pub mod transport;
pub mod url;

mod files;

pub use coordinator::{
    AdoptReport, Coordinator, CoordinatorConfig, CreatedProfile, JoinOutcome, SharedProfiles,
    SyncStatus, VaultHost,
};
pub use engine::{sha256_hex, JoinPreparation, PullResult, PushResult, SyncEngine};
pub use error::SyncError;
pub use profile::{MemoryTokenStore, SyncProfile, SyncProfileStore, TokenStore};
pub use runner::SyncHandle;
pub use time::{Clock, SystemClock};
pub use transport::{FetchResult, HttpTransport, SlotState, Transport, VersionInfo};
pub use url::normalize_server_url;
