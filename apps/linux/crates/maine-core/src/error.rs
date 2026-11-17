use std::fmt;

/// Why a vault operation failed. Wording matches the macOS client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultError {
    Locked,
    InvalidPassword,
    InvalidFile,
    WeakPassword,
    /// A vault format this client does not read (e.g. v1, which macOS migrates).
    UnsupportedVersion(u32),
    Io(String),
}

impl fmt::Display for VaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Locked => f.write_str("The password vault is locked. Unlock it with its master password."),
            Self::InvalidPassword => f.write_str("The master password is incorrect, or the vault has been damaged."),
            Self::InvalidFile => f.write_str(
                "This password vault is damaged or uses an unsupported format. It has not been changed.",
            ),
            Self::WeakPassword => f.write_str("Use a master password of at least 12 characters."),
            Self::UnsupportedVersion(v) => write!(
                f,
                "This vault uses format version {v}, which this client cannot read. Unlock it once on macOS to upgrade it."
            ),
            Self::Io(message) => write!(f, "Could not access the vault file: {message}"),
        }
    }
}

impl std::error::Error for VaultError {}

impl From<std::io::Error> for VaultError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// A user-facing failure from layout or host-store operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreError(pub String);

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_messages_match_macos() {
        assert_eq!(
            VaultError::InvalidPassword.to_string(),
            "The master password is incorrect, or the vault has been damaged."
        );
        assert_eq!(
            VaultError::WeakPassword.to_string(),
            "Use a master password of at least 12 characters."
        );
        assert_eq!(
            VaultError::InvalidFile.to_string(),
            "This password vault is damaged or uses an unsupported format. It has not been changed."
        );
    }
}
