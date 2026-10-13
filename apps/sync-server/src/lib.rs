//! Maine sync server: a zero-knowledge, single-token store for the encrypted
//! `passwords.maine` vault.
//!
//! The server never sees plaintext, a master password, or a token: a slot is
//! addressed by the SHA-256 of its token, and its contents are opaque bytes.
//! The wire contract lives in `docs/sync-protocol.md` (repository root).

pub mod api;
pub mod config;
pub mod db;
pub mod hash;
pub mod token;
