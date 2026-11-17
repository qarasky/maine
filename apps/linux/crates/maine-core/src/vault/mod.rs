//! Portable AES-256-GCM vault (`passwords.maine`, format v2), byte-compatible
//! with the macOS and Android clients.

mod contents;
mod envelope;
mod file;
mod local;

pub use contents::{Contents, SavedSession};
pub use envelope::{context, derive_key, open, seal, VaultKey, ITERATIONS, MARKER};
pub use file::{Unlocked, VaultFile};
pub use local::LocalState;
