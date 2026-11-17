use crate::model::uuid_upper::upper;
use crate::VaultError;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use rand::RngCore;
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const ITERATIONS: u32 = 600_000;
pub const MARKER: &[u8] = b"Maine password vault v1";
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

/// A derived 32-byte vault key. Zeroed on drop; never printed.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct VaultKey([u8; 32]);

impl VaultKey {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for VaultKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VaultKey(<redacted>)")
    }
}

/// PBKDF2-HMAC-SHA256 over the raw UTF-8 bytes of the password (no normalization).
pub fn derive_key(password: &str, salt: &[u8], iterations: u32) -> VaultKey {
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password.as_bytes(), salt, iterations, &mut key);
    VaultKey(key)
}

/// The AAD every sealed blob of a vault is bound to.
pub fn context(version: u32, id: Uuid, salt: &[u8], iterations: u32) -> Vec<u8> {
    format!(
        "maine-vault:{version}:{}:{}:{iterations}",
        upper(&id),
        STANDARD.encode(salt)
    )
    .into_bytes()
}

/// `nonce(12) | ciphertext | tag(16)`, random nonce.
pub fn seal(plain: &[u8], key: &VaultKey, aad: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new(key.as_bytes().into());
    let mut nonce = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce);
    let sealed = cipher
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: plain, aad })
        .expect("AES-GCM encryption of in-memory data cannot fail");
    let mut out = Vec::with_capacity(NONCE_LEN + sealed.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    out
}

pub fn open(sealed: &[u8], key: &VaultKey, aad: &[u8]) -> Result<Vec<u8>, VaultError> {
    if sealed.len() < NONCE_LEN + TAG_LEN {
        return Err(VaultError::InvalidFile);
    }
    let (nonce, rest) = sealed.split_at(NONCE_LEN);
    Aes256Gcm::new(key.as_bytes().into())
        .decrypt(Nonce::from_slice(nonce), Payload { msg: rest, aad })
        .map_err(|_| VaultError::InvalidPassword)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_format_matches_macos() {
        let id = Uuid::parse_str("6AA6118A-7CB3-4F6E-BBED-00F0E2972F2F").unwrap();
        let salt = STANDARD
            .decode("nD0Ue790c9S04xu2nWg/Tsfb20ejbbpFVY2Bo7nZ7x0=")
            .unwrap();
        assert_eq!(
            String::from_utf8(context(2, id, &salt, 600_000)).unwrap(),
            "maine-vault:2:6AA6118A-7CB3-4F6E-BBED-00F0E2972F2F:nD0Ue790c9S04xu2nWg/Tsfb20ejbbpFVY2Bo7nZ7x0=:600000"
        );
    }

    fn key() -> VaultKey {
        derive_key("correct horse battery", &[7u8; 32], 1000)
    }

    #[test]
    fn seal_open_roundtrip() {
        let sealed = seal(b"hello", &key(), b"aad");
        assert_eq!(sealed.len(), 12 + 5 + 16);
        assert_eq!(open(&sealed, &key(), b"aad").unwrap(), b"hello");
    }

    #[test]
    fn open_with_wrong_aad_fails() {
        let sealed = seal(b"hello", &key(), b"aad");
        assert_eq!(
            open(&sealed, &key(), b"other"),
            Err(VaultError::InvalidPassword)
        );
    }

    #[test]
    fn open_truncated_fails() {
        assert_eq!(open(&[0u8; 27], &key(), b""), Err(VaultError::InvalidFile));
    }

    #[test]
    fn derive_key_uses_raw_utf8_bytes() {
        let nfc = derive_key("\u{e9}", &[1u8; 32], 10);
        let nfd = derive_key("e\u{301}", &[1u8; 32], 10);
        assert_ne!(nfc.as_bytes(), nfd.as_bytes());
        assert_eq!(
            derive_key("x", &[1u8; 32], 10).as_bytes(),
            derive_key("x", &[1u8; 32], 10).as_bytes()
        );
    }
}
