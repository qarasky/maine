use std::fmt;

/// Everything that can go wrong in the sync client. Transport failures,
/// engine refusals and local file problems share one type so callers (and the
/// coordinator) match on a single enum. Messages are user-facing and match the
/// macOS client; none of them ever contains a token, a URL or file contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    // Transport (docs/sync-protocol.md status codes).
    /// `401`.
    Unauthorized,
    /// `403`, with the server's message.
    Forbidden(String),
    /// `404`, with the server's message.
    NotFound(String),
    /// `409` or `412`: the slot is not at the version the client expected.
    Conflict {
        current_version: u64,
    },
    /// `428`: a write to a non-empty slot without a precondition.
    PreconditionRequired {
        current_version: u64,
    },
    /// `413`.
    TooLarge,
    /// DNS, connect, TLS or timeout failures.
    Offline(String),
    /// Any other status (`5xx`, unexpected `3xx`...).
    Server {
        status: u16,
        message: String,
    },
    /// A response the client could not understand.
    Malformed(String),

    // Engine refusals. Each keeps the downloaded bytes as a `remote` copy.
    NoLocalVault,
    WrongMasterPassword {
        remote_copy: String,
    },
    UnreadableRemoteVault {
        remote_copy: String,
    },
    ForeignVault {
        remote_copy: String,
    },
    ShaMismatch {
        remote_copy: String,
    },

    // Local state.
    /// The secret store (Secret Service on Linux) failed.
    TokenStore(String),
    /// A vault, copy or workspace file could not be read or written.
    Io(String),
    /// The profile list could not be saved.
    Profiles(String),
    /// The app (the `VaultHost`) refused or failed a step, with its message.
    Host(String),
}

impl SyncError {
    /// Refusals from the engine that stop syncing until the user acts.
    pub fn is_engine_refusal(&self) -> bool {
        matches!(
            self,
            Self::NoLocalVault
                | Self::WrongMasterPassword { .. }
                | Self::UnreadableRemoteVault { .. }
                | Self::ForeignVault { .. }
                | Self::ShaMismatch { .. }
        )
    }
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized => f.write_str("The server did not accept this sync token."),
            Self::Forbidden(message) | Self::NotFound(message) | Self::Offline(message) => {
                f.write_str(message)
            }
            Self::Conflict { .. } => f.write_str("Another device changed the vault first."),
            Self::PreconditionRequired { .. } => {
                f.write_str("The sync client sent an incomplete write request.")
            }
            Self::TooLarge => f.write_str("The vault is larger than this server accepts."),
            Self::Server { message, .. } | Self::Malformed(message) => f.write_str(message),
            Self::NoLocalVault => f.write_str("There is no local vault to sync yet."),
            Self::WrongMasterPassword { remote_copy } => write!(
                f,
                "The master password did not open the vault on the server. Nothing was changed; the downloaded copy is {remote_copy}."
            ),
            Self::UnreadableRemoteVault { remote_copy } => write!(
                f,
                "The server holds a vault this version of Maine cannot read. Update the app; the downloaded copy is {remote_copy}."
            ),
            Self::ForeignVault { remote_copy } => write!(
                f,
                "The vault on the server belongs to a different master password. Nothing was changed; the downloaded copy is {remote_copy}."
            ),
            Self::ShaMismatch { remote_copy } => write!(
                f,
                "The server's vault does not match its checksum. It was kept as {remote_copy}; nothing was changed."
            ),
            Self::TokenStore(message) => write!(f, "Secret storage: {message}"),
            Self::Io(message) => write!(f, "Could not access the sync files: {message}"),
            Self::Profiles(message) => write!(f, "Could not save the sync profiles: {message}"),
            Self::Host(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SyncError {}

impl From<std::io::Error> for SyncError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
