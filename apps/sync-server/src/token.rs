//! Token generation, normalization and hashing.
//!
//! A token is 32 random bytes rendered with a 32-character, unambiguous
//! alphabet, displayed as `maine-XXXX-XXXX-...`. It is both the address of a
//! slot and the credential that unlocks it, so the server only ever stores its
//! SHA-256. Losing a token means losing access to the slot; there is no reset.

use crate::hash;

/// No `0`, `1`, `I` or `O`, so a pasted token cannot be misread.
const ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";

/// Canonical prefix. The random core cannot contain an `I`, so stripping a
/// leading `MAINE` during normalization is unambiguous.
pub const PREFIX: &str = "MAINE";

/// Length of the random core in characters: 256 bits padded to 52 * 5 bits.
pub const CORE_LEN: usize = 52;

/// Bytes of randomness behind every token.
const CORE_BYTES: usize = 32;

/// Generate a new token's canonical core (52 characters, no prefix).
pub fn generate_core() -> String {
    let mut bytes = [0u8; CORE_BYTES];
    getrandom::fill(&mut bytes).expect("the OS random generator must be available");
    encode(&bytes)
}

/// The canonical form of a freshly generated token: `MAINE` plus the core.
pub fn generate() -> String {
    let mut canonical = String::with_capacity(PREFIX.len() + CORE_LEN);
    canonical.push_str(PREFIX);
    canonical.push_str(&generate_core());
    canonical
}

/// Base32-ish encoding, most significant bits first, zero padded at the end.
fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(CORE_LEN);
    let mut accumulator: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in bytes {
        accumulator = (accumulator << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((accumulator >> bits) & 0b11111) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((accumulator << (5 - bits)) & 0b11111) as usize] as char);
    }
    out
}

/// Normalize a user-supplied token, or return `None` when it cannot be one.
///
/// Tolerates the display form: separators, surrounding whitespace and case all
/// disappear, and the optional `maine-` prefix is removed.
pub fn normalize(input: &str) -> Option<String> {
    let filtered: String = input
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let core = filtered.strip_prefix(PREFIX).unwrap_or(&filtered);
    if core.len() != CORE_LEN || !core.bytes().all(|byte| ALPHABET.contains(&byte)) {
        return None;
    }
    let mut canonical = String::with_capacity(PREFIX.len() + CORE_LEN);
    canonical.push_str(PREFIX);
    canonical.push_str(core);
    Some(canonical)
}

/// Display form for humans and UI: `maine-XXXX-XXXX-...`, grouped in fours.
pub fn display(canonical: &str) -> String {
    let core = canonical.strip_prefix(PREFIX).unwrap_or(canonical);
    let mut out = String::with_capacity(core.len() + core.len() / 4 + 6);
    out.push_str("maine");
    for chunk in core.as_bytes().chunks(4) {
        out.push('-');
        out.push_str(std::str::from_utf8(chunk).expect("the token core is ASCII"));
    }
    out
}

/// The lookup hash for a canonical token. Never store or log anything else.
pub fn hash_of(canonical: &str) -> [u8; 32] {
    hash::sha256(canonical.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_core_has_the_expected_shape() {
        let core = generate_core();
        assert_eq!(core.len(), CORE_LEN);
        assert!(core.bytes().all(|byte| ALPHABET.contains(&byte)));
    }

    #[test]
    fn display_round_trips_through_normalize() {
        let canonical = generate();
        let display = display(&canonical);
        assert!(display.starts_with("maine-"));
        assert_eq!(normalize(&display).as_deref(), Some(canonical.as_str()));
    }

    #[test]
    fn normalize_tolerates_case_spacing_and_missing_prefix() {
        let canonical = generate();
        let core = canonical.strip_prefix(PREFIX).unwrap();
        let sloppy = format!(
            "  {} {}  ",
            core[..4].to_lowercase(),
            core[4..].to_lowercase()
        );
        assert_eq!(normalize(&sloppy).as_deref(), Some(canonical.as_str()));
    }

    #[test]
    fn normalize_rejects_garbage() {
        assert!(normalize("").is_none());
        assert!(normalize("maine").is_none());
        assert!(normalize("maine-0000-0000").is_none());
        // I, O, 0 and 1 are not in the alphabet.
        assert!(normalize("MAINEIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIII").is_none());
    }

    #[test]
    fn hash_is_stable_and_keyed_by_the_canonical_form() {
        let canonical = generate();
        assert_eq!(hash_of(&canonical), hash_of(&canonical));
        let other = generate();
        assert_ne!(hash_of(&canonical), hash_of(&other));
    }
}
