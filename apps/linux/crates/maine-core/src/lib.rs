//! Qt-free core of the Maine Linux client. Mirrors the macOS `Models/` and
//! `Services/` layers; nothing here knows a UI toolkit exists.
#![forbid(unsafe_code)]

pub mod error;
pub mod model;
pub mod ssh;
pub mod vault;

pub use error::{StoreError, VaultError};
