//! Small hashing helpers shared by tokens and stored blobs.

use sha2::{Digest, Sha256};

/// SHA-256 as a plain 32-byte array, so callers never carry digest types around.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest[..]);
    out
}

/// Lowercase hex, for JSON and headers.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // Writing into a String cannot fail.
        let _ = write!(out, "{byte:02x}");
    }
    out
}
