//! Token forms from `docs/sync-protocol.md`. The client sends what the user
//! pasted (trimmed); these helpers exist for validation and display only.
//! Tokens are secrets: nothing here logs or formats one into an error.

const ALPHABET: &str = "23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
const PREFIX: &str = "MAINE";
const CORE_LEN: usize = 52;

/// The canonical form (`MAINE` plus 52 characters) of any accepted spelling:
/// separators, case and whitespace disappear and the `maine-` prefix is
/// optional. `None` when the text cannot be a token.
pub fn normalize_token(input: &str) -> Option<String> {
    let filtered: String = input
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let core = filtered.strip_prefix(PREFIX).unwrap_or(&filtered);
    if core.len() != CORE_LEN || !core.chars().all(|c| ALPHABET.contains(c)) {
        return None;
    }
    Some(format!("{PREFIX}{core}"))
}

/// `maine-XXXX-XXXX-...` grouped in fours, from any accepted spelling.
pub fn display_token(input: &str) -> Option<String> {
    let canonical = normalize_token(input)?;
    let core = canonical.strip_prefix(PREFIX)?;
    let mut out = String::from("maine");
    for chunk in core.as_bytes().chunks(4) {
        out.push('-');
        out.push_str(std::str::from_utf8(chunk).ok()?);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORE: &str = "2F7K9QX23456789ABCDEFGHJKLMNPQRSTUVWXYZ23456789ABCDE";

    #[test]
    fn core_constant_has_the_right_shape() {
        assert_eq!(CORE.len(), CORE_LEN);
    }

    #[test]
    fn display_and_normalize_round_trip() {
        let display = display_token(CORE).unwrap();
        assert!(display.starts_with("maine-2F7K-9QX2-"));
        assert_eq!(display.matches('-').count(), 13);
        assert_eq!(
            normalize_token(&display).as_deref(),
            Some(format!("MAINE{CORE}").as_str())
        );
    }

    #[test]
    fn normalize_tolerates_case_spaces_and_missing_prefix() {
        let sloppy = format!(
            "  {} {}  ",
            CORE[..4].to_lowercase(),
            CORE[4..].to_lowercase()
        );
        assert_eq!(
            normalize_token(&sloppy).as_deref(),
            Some(format!("MAINE{CORE}").as_str())
        );
    }

    #[test]
    fn normalize_rejects_wrong_length_and_alphabet() {
        assert_eq!(normalize_token("maine-2F7K"), None);
        assert_eq!(normalize_token(&format!("{}0", &CORE[..51])), None);
        assert_eq!(normalize_token(""), None);
    }
}
