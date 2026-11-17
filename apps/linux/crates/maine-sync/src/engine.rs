//! The client-side sync algorithms from `docs/sync-client.md`: whole-file, byte
//! for byte, with a conflict copy whenever a local edit would be lost. The
//! engine is files plus one [`Transport`]; it never decrypts or re-encrypts
//! the vault, and it knows nothing about a UI or a scheduler.

use crate::error::SyncError;
use crate::files::{unique_name, write_atomic, write_new_copy};
use crate::time::file_timestamp_now;
use crate::transport::{FetchResult, SlotState, Transport, VersionInfo};
use maine_core::vault::{Unlocked, VaultFile};
use maine_core::VaultError;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

/// Lowercase hex SHA-256, the form of `X-Maine-SHA256` and `lastSHA256`.
pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinPreparation {
    EmptySlot,
    /// Validated remote bytes, ready to adopt once the caller has parked
    /// whatever vault is live now.
    Ready(Vec<u8>, SlotState),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullResult {
    NotModified(SlotState),
    Empty(SlotState),
    /// The remote blob is now the live vault. `conflict_copy` is the file
    /// name of the dirty local vault saved beside it, if there was one.
    Adopted {
        state: SlotState,
        conflict_copy: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushResult {
    /// The file already matches the last synced version.
    Skipped(SlotState),
    Stored(SlotState),
}

type Timestamp = Box<dyn Fn() -> String + Send + Sync>;

pub struct SyncEngine<T: Transport> {
    vault_path: PathBuf,
    transport: T,
    timestamp: Timestamp,
}

impl<T: Transport> SyncEngine<T> {
    pub fn new(vault_path: impl Into<PathBuf>, transport: T) -> Self {
        Self {
            vault_path: vault_path.into(),
            transport,
            timestamp: Box::new(file_timestamp_now),
        }
    }

    /// Replaces the `YYYYMMDD-HHMMSS` source used in copy names (UTC by
    /// default; tests and local-time callers inject their own).
    pub fn with_timestamp(
        mut self,
        timestamp: impl Fn() -> String + Send + Sync + 'static,
    ) -> Self {
        self.timestamp = Box::new(timestamp);
        self
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn vault_path(&self) -> &Path {
        &self.vault_path
    }

    fn vault_dir(&self) -> &Path {
        self.vault_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
    }

    // MARK: - Local files

    pub fn local_vault_exists(&self) -> bool {
        self.vault_path.exists()
    }

    pub fn local_vault_data(&self) -> Result<Vec<u8>, SyncError> {
        match fs::read(&self.vault_path) {
            Ok(data) => Ok(data),
            Err(error) if error.kind() == ErrorKind::NotFound => Err(SyncError::NoLocalVault),
            Err(error) => Err(error.into()),
        }
    }

    /// Hex SHA-256 of the live vault, or `None` when there is none.
    pub fn local_vault_sha256(&self) -> Option<String> {
        fs::read(&self.vault_path)
            .ok()
            .map(|data| sha256_hex(&data))
    }

    /// `passwords.workspace-<PROFILE-ID>.maine` next to the live vault.
    pub fn workspace_path(&self, profile: Uuid) -> PathBuf {
        self.vault_dir().join(format!(
            "passwords.workspace-{}.maine",
            profile.hyphenated().to_string().to_uppercase()
        ))
    }

    fn copy_name(&self, kind: &str) -> String {
        format!("passwords.{kind}-{}.maine", (self.timestamp)())
    }

    /// Saves a copy next to the vault (`0600`, never replacing an existing
    /// file) and returns its file name. Used for the pre-sync, remote and
    /// conflict cases; copies are ordinary vault files and sync never deletes
    /// them.
    pub fn save_copy(&self, data: &[u8], kind: &str) -> Result<String, SyncError> {
        Ok(write_new_copy(
            self.vault_dir(),
            &self.copy_name(kind),
            data,
        )?)
    }

    /// Copies the live vault to `destination` (`0600`), leaving the live file
    /// in place. Nothing happens when there is no live vault.
    pub fn copy_local_vault(&self, destination: &Path) -> Result<(), SyncError> {
        match self.local_vault_data() {
            Ok(data) => Ok(write_atomic(destination, &data)?),
            Err(SyncError::NoLocalVault) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Moves the live vault to `destination` when it exists, replacing a file
    /// already there. The rename is atomic; there is no moment with neither.
    pub fn move_local_vault(&self, destination: &Path) -> Result<(), SyncError> {
        if !self.local_vault_exists() {
            return Ok(());
        }
        Ok(fs::rename(&self.vault_path, destination)?)
    }

    /// Moves a live vault aside as `passwords.<kind>-<timestamp>.maine`.
    pub fn move_local_vault_aside(&self, kind: &str) -> Result<Option<String>, SyncError> {
        if !self.local_vault_exists() {
            return Ok(None);
        }
        let name = unique_name(self.vault_dir(), &self.copy_name(kind));
        self.move_local_vault(&self.vault_dir().join(&name))?;
        Ok(Some(name))
    }

    /// Moves a workspace file into place as the live vault (atomic replace).
    pub fn adopt_local_vault(&self, source: &Path) -> Result<(), SyncError> {
        if !source.exists() {
            return Err(SyncError::NoLocalVault);
        }
        fs::rename(source, &self.vault_path)?;
        fs::set_permissions(&self.vault_path, fs::Permissions::from_mode(0o600))?;
        Ok(())
    }

    fn atomic_replace(&self, data: &[u8]) -> Result<(), SyncError> {
        Ok(write_atomic(&self.vault_path, data)?)
    }

    fn check_sha(&self, bytes: &[u8], state: &SlotState) -> Result<(), SyncError> {
        match &state.sha256 {
            Some(expected) if *expected != sha256_hex(bytes) => Err(SyncError::ShaMismatch {
                remote_copy: self.save_copy(bytes, "remote")?,
            }),
            _ => Ok(()),
        }
    }

    // MARK: - Create

    /// Uploads the current vault as the first version of a new slot (an empty
    /// body when there is no vault). The server returns the token exactly
    /// once; persist it before showing success.
    pub fn create(
        &self,
        server: &str,
        create_secret: Option<&str>,
    ) -> Result<(String, SlotState), SyncError> {
        let body = match self.local_vault_data() {
            Ok(data) => data,
            Err(SyncError::NoLocalVault) => Vec::new(),
            Err(error) => return Err(error),
        };
        self.transport.create_vault(server, create_secret, &body)
    }

    // MARK: - Join

    /// Downloads and validates the remote vault without changing any live
    /// file. A refused download is kept as `passwords.remote-<timestamp>.maine`.
    pub fn prepare_join(
        &self,
        server: &str,
        token: &str,
        master_password: &str,
    ) -> Result<JoinPreparation, SyncError> {
        match self.transport.fetch_vault(server, token, None)? {
            FetchResult::Empty(_) => Ok(JoinPreparation::EmptySlot),
            FetchResult::NotModified(_) | FetchResult::Current(_) => Err(SyncError::Malformed(
                "The server answered without the vault body.".into(),
            )),
            FetchResult::Blob(bytes, state) => {
                self.check_sha(&bytes, &state)?;
                match VaultFile::validate_blob(&bytes, master_password) {
                    Ok(()) => Ok(JoinPreparation::Ready(bytes, state)),
                    Err(VaultError::InvalidPassword) => Err(SyncError::WrongMasterPassword {
                        remote_copy: self.save_copy(&bytes, "remote")?,
                    }),
                    Err(_) => Err(SyncError::UnreadableRemoteVault {
                        remote_copy: self.save_copy(&bytes, "remote")?,
                    }),
                }
            }
        }
    }

    /// Replaces the live vault with previously validated bytes. A local vault
    /// that is still present is moved to `passwords.pre-sync-<timestamp>.maine`
    /// first.
    pub fn adopt_prepared_join(&self, bytes: &[u8], state: &SlotState) -> Result<(), SyncError> {
        if state.version == 0 {
            return Err(SyncError::Malformed("The slot reported version 0.".into()));
        }
        self.move_local_vault_aside("pre-sync")?;
        self.atomic_replace(bytes)
    }

    // MARK: - Pull

    /// `GET` with `If-None-Match`. A `200` is adopted only when its checksum
    /// matches and `unlocked`'s key opens it as this same vault; a dirty local
    /// vault is saved as a conflict copy first.
    pub fn pull(
        &self,
        server: &str,
        token: &str,
        last_version: u64,
        is_dirty: bool,
        unlocked: &Unlocked,
    ) -> Result<PullResult, SyncError> {
        match self
            .transport
            .fetch_vault(server, token, Some(last_version))?
        {
            FetchResult::NotModified(state) => Ok(PullResult::NotModified(state)),
            FetchResult::Empty(state) => Ok(PullResult::Empty(state)),
            FetchResult::Current(_) => Err(SyncError::Malformed(
                "The server answered without the vault body.".into(),
            )),
            FetchResult::Blob(bytes, state) => {
                self.check_sha(&bytes, &state)?;
                if !VaultFile::key_opens(&bytes, unlocked) {
                    return Err(SyncError::ForeignVault {
                        remote_copy: self.save_copy(&bytes, "remote")?,
                    });
                }
                let mut conflict_copy = None;
                if is_dirty {
                    match self.local_vault_data() {
                        Ok(local) => conflict_copy = Some(self.save_copy(&local, "conflict")?),
                        Err(SyncError::NoLocalVault) => {}
                        // An unreadable local file must not be replaced unseen.
                        Err(error) => return Err(error),
                    }
                }
                self.atomic_replace(&bytes)?;
                Ok(PullResult::Adopted {
                    state,
                    conflict_copy,
                })
            }
        }
    }

    /// A cheap `HEAD` used while polling; `304` almost always.
    pub fn head(
        &self,
        server: &str,
        token: &str,
        last_version: u64,
    ) -> Result<FetchResult, SyncError> {
        self.transport.head_vault(server, token, Some(last_version))
    }

    // MARK: - Push

    /// Uploads the live vault, skipping the request when its hash equals the
    /// one recorded at `last_version`. `If-None-Match: *` for version 0,
    /// otherwise `If-Match`. `timeout` bounds this one request.
    pub fn push(
        &self,
        server: &str,
        token: &str,
        last_version: u64,
        last_sha256: Option<&str>,
        timeout: Option<Duration>,
    ) -> Result<PushResult, SyncError> {
        let bytes = self.local_vault_data()?;
        let sha = sha256_hex(&bytes);
        if last_sha256 == Some(sha.as_str()) {
            return Ok(PushResult::Skipped(SlotState::with(
                last_version,
                bytes.len() as u64,
                sha,
            )));
        }
        let expected = (last_version > 0).then_some(last_version);
        let state = self
            .transport
            .put_vault(server, token, &bytes, expected, timeout)?;
        Ok(PushResult::Stored(state))
    }

    // MARK: - Token and slot management

    pub fn rotate(&self, server: &str, token: &str) -> Result<String, SyncError> {
        self.transport.rotate_token(server, token)
    }

    pub fn delete(&self, server: &str, token: &str) -> Result<(), SyncError> {
        self.transport.delete_vault(server, token)
    }

    pub fn list_versions(&self, server: &str, token: &str) -> Result<Vec<VersionInfo>, SyncError> {
        self.transport.list_versions(server, token)
    }

    /// Downloads an old version and keeps it as
    /// `passwords.remote-<timestamp>.maine` for the user to open; the live
    /// vault is untouched. Returns the copy's file name.
    pub fn save_version_copy(
        &self,
        server: &str,
        token: &str,
        version: u64,
    ) -> Result<String, SyncError> {
        let (bytes, state) = self.transport.fetch_version(server, token, version)?;
        self.check_sha(&bytes, &state)?;
        self.save_copy(&bytes, "remote")
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
