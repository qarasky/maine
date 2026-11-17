//! Sync profiles (non-secret) and the token store boundary.
//!
//! A profile is the client-side record of one sync pairing. Its token is not
//! part of the record: it lives in the OS secret store behind [`TokenStore`],
//! keyed by the profile id, so it survives a remote vault replacing the local
//! file and never reaches the profile JSON, the vault or a log.

use crate::error::SyncError;
use crate::files::{write_atomic, write_new_copy};
use crate::time::file_timestamp_now;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// File name of the profile list inside the caller-supplied directory.
pub const PROFILES_FILE_NAME: &str = "sync-profiles.json";

/// One paired sync slot as remembered on this device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncProfile {
    pub id: Uuid,
    pub name: String,
    /// Normalized base URL (see `normalize_server_url`).
    #[serde(rename = "serverURL")]
    pub server_url: String,
    /// The version the local vault came from. `0` means never synced.
    #[serde(rename = "lastVersion", default)]
    pub last_version: u64,
    /// Hex SHA-256 of the file at `last_version`, used to skip no-op pushes.
    #[serde(
        rename = "lastSHA256",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_sha256: Option<String>,
    /// Seconds since 2001-01-01 UTC (Swift's `Date` encoding); UI only.
    #[serde(
        rename = "lastSyncedAt",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_synced_at: Option<f64>,
}

impl SyncProfile {
    pub fn new(name: impl Into<String>, server_url: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            server_url: server_url.into(),
            last_version: 0,
            last_sha256: None,
            last_synced_at: None,
        }
    }
}

/// Where tokens live. The app backs this with the Secret Service; tests use
/// [`MemoryTokenStore`]. Implementations must never log a token.
pub trait TokenStore: Send + Sync {
    fn get(&self, profile: Uuid) -> Result<Option<String>, SyncError>;
    fn set(&self, profile: Uuid, token: &str) -> Result<(), SyncError>;
    /// Deleting a token that is not there is not an error.
    fn delete(&self, profile: Uuid) -> Result<(), SyncError>;
}

impl<T: TokenStore + ?Sized> TokenStore for Arc<T> {
    fn get(&self, profile: Uuid) -> Result<Option<String>, SyncError> {
        (**self).get(profile)
    }
    fn set(&self, profile: Uuid, token: &str) -> Result<(), SyncError> {
        (**self).set(profile, token)
    }
    fn delete(&self, profile: Uuid) -> Result<(), SyncError> {
        (**self).delete(profile)
    }
}

/// In-process token store for tests. It does not persist and does not hide
/// anything, so never use it in the app.
#[derive(Default)]
pub struct MemoryTokenStore {
    tokens: Mutex<HashMap<Uuid, String>>,
}

impl MemoryTokenStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for MemoryTokenStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MemoryTokenStore(<redacted>)")
    }
}

impl TokenStore for MemoryTokenStore {
    fn get(&self, profile: Uuid) -> Result<Option<String>, SyncError> {
        Ok(self.tokens.lock().unwrap().get(&profile).cloned())
    }
    fn set(&self, profile: Uuid, token: &str) -> Result<(), SyncError> {
        self.tokens
            .lock()
            .unwrap()
            .insert(profile, token.to_string());
        Ok(())
    }
    fn delete(&self, profile: Uuid) -> Result<(), SyncError> {
        self.tokens.lock().unwrap().remove(&profile);
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct ProfilesFile {
    version: u32,
    #[serde(default)]
    profiles: Vec<SyncProfile>,
    #[serde(rename = "activeProfile", default)]
    active_profile: Option<Uuid>,
}

/// The remembered pairings and their tokens. Exactly one profile is active at
/// a time; the active profile's vault is the live `passwords.maine`.
pub struct SyncProfileStore {
    path: PathBuf,
    tokens: Arc<dyn TokenStore>,
    profiles: Vec<SyncProfile>,
    active: Option<Uuid>,
    recovered_backup: Option<String>,
}

impl SyncProfileStore {
    /// Opens (or starts empty in) `directory`. A missing file is an empty
    /// list. A file that cannot be parsed is copied aside as
    /// `sync-profiles.json.corrupt-<timestamp>` (never deleted) and the list
    /// starts empty; see [`recovered_backup`](Self::recovered_backup).
    pub fn open(
        directory: impl AsRef<Path>,
        tokens: Arc<dyn TokenStore>,
    ) -> Result<Self, SyncError> {
        let dir = directory.as_ref();
        let path = dir.join(PROFILES_FILE_NAME);
        let mut store = Self {
            path,
            tokens,
            profiles: Vec::new(),
            active: None,
            recovered_backup: None,
        };
        match fs::read(&store.path) {
            Ok(bytes) => match serde_json::from_slice::<ProfilesFile>(&bytes) {
                Ok(file) => {
                    store.profiles = file.profiles;
                    store.active = file
                        .active_profile
                        .filter(|id| store.profiles.iter().any(|p| p.id == *id));
                }
                Err(_) => {
                    let name = format!("{PROFILES_FILE_NAME}.corrupt-{}", file_timestamp_now());
                    store.recovered_backup = Some(write_new_copy(dir, &name, &bytes)?);
                }
            },
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Set when the profile file was unreadable and a copy of it was kept.
    pub fn recovered_backup(&self) -> Option<&str> {
        self.recovered_backup.as_deref()
    }

    pub fn profiles(&self) -> &[SyncProfile] {
        &self.profiles
    }

    pub fn has_profiles(&self) -> bool {
        !self.profiles.is_empty()
    }

    pub fn active_profile_id(&self) -> Option<Uuid> {
        self.active
    }

    pub fn active_profile(&self) -> Option<&SyncProfile> {
        self.active.and_then(|id| self.profile(id))
    }

    pub fn profile(&self, id: Uuid) -> Option<&SyncProfile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    pub fn token(&self, id: Uuid) -> Result<Option<String>, SyncError> {
        self.tokens.get(id)
    }

    /// Stores the token first, then the profile; the first profile becomes
    /// active. If the list cannot be saved the token is removed again.
    pub fn add(&mut self, profile: SyncProfile, token: &str) -> Result<(), SyncError> {
        if self.profile(profile.id).is_some() {
            return Err(SyncError::Profiles("That profile already exists.".into()));
        }
        self.tokens.set(profile.id, token)?;
        let mut profiles = self.profiles.clone();
        let id = profile.id;
        profiles.push(profile);
        let active = self.active.or(Some(id));
        if let Err(error) = self.commit(profiles, active) {
            let _ = self.tokens.delete(id);
            return Err(error);
        }
        Ok(())
    }

    /// Applies `change` to one profile and saves. `Ok(false)` when the id is
    /// unknown.
    pub fn update(
        &mut self,
        id: Uuid,
        change: impl FnOnce(&mut SyncProfile),
    ) -> Result<bool, SyncError> {
        let mut profiles = self.profiles.clone();
        let Some(profile) = profiles.iter_mut().find(|p| p.id == id) else {
            return Ok(false);
        };
        change(profile);
        self.commit(profiles, self.active)?;
        Ok(true)
    }

    /// Persists a rotated token before anything acknowledges the rotation.
    pub fn replace_token(&self, token: &str, id: Uuid) -> Result<(), SyncError> {
        self.tokens.set(id, token)
    }

    /// Forgets this device's half of a pairing: the token, then the profile.
    /// Nothing on the server changes and workspace files stay on disk.
    pub fn remove(&mut self, id: Uuid) -> Result<(), SyncError> {
        self.tokens.delete(id)?;
        let profiles: Vec<_> = self
            .profiles
            .iter()
            .filter(|p| p.id != id)
            .cloned()
            .collect();
        let active = self.active.filter(|active| *active != id);
        self.commit(profiles, active)
    }

    pub fn set_active(&mut self, id: Option<Uuid>) -> Result<(), SyncError> {
        if let Some(id) = id {
            if self.profile(id).is_none() {
                return Err(SyncError::Profiles("That profile does not exist.".into()));
            }
        }
        self.commit(self.profiles.clone(), id)
    }

    /// Writes the new state to disk, then adopts it in memory, so a failed
    /// write leaves memory and disk in step.
    fn commit(
        &mut self,
        profiles: Vec<SyncProfile>,
        active: Option<Uuid>,
    ) -> Result<(), SyncError> {
        let file = ProfilesFile {
            version: 1,
            profiles,
            active_profile: active,
        };
        let bytes = serde_json::to_vec_pretty(&file)
            .map_err(|error| SyncError::Profiles(error.to_string()))?;
        write_atomic(&self.path, &bytes).map_err(|error| SyncError::Profiles(error.to_string()))?;
        self.profiles = file.profiles;
        self.active = file.active_profile;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn store(dir: &Path, tokens: &Arc<MemoryTokenStore>) -> SyncProfileStore {
        SyncProfileStore::open(dir, tokens.clone()).unwrap()
    }

    fn profile() -> SyncProfile {
        SyncProfile::new("Homelab", "http://127.0.0.1:8484")
    }

    /// Port of `profilesAndTokensPersistAndRemoveCleanly`.
    #[test]
    fn profiles_and_tokens_persist_and_remove_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let tokens = Arc::new(MemoryTokenStore::new());

        let mut first = store(dir.path(), &tokens);
        let profile = profile();
        first.add(profile.clone(), "maine-TEST-TOKEN").unwrap();

        assert_eq!(first.profiles(), std::slice::from_ref(&profile));
        assert_eq!(first.active_profile_id(), Some(profile.id));
        assert_eq!(
            first.token(profile.id).unwrap().as_deref(),
            Some("maine-TEST-TOKEN")
        );

        // A second store instance sees the same record and token.
        let mut reopened = store(dir.path(), &tokens);
        assert_eq!(reopened.profiles(), std::slice::from_ref(&profile));
        assert_eq!(reopened.active_profile_id(), Some(profile.id));
        assert_eq!(
            reopened.token(profile.id).unwrap().as_deref(),
            Some("maine-TEST-TOKEN")
        );

        // Rotation replaces the token; removal forgets both halves.
        reopened
            .replace_token("maine-NEW-TOKEN", profile.id)
            .unwrap();
        assert_eq!(
            reopened.token(profile.id).unwrap().as_deref(),
            Some("maine-NEW-TOKEN")
        );

        reopened.remove(profile.id).unwrap();
        assert!(reopened.profiles().is_empty());
        assert_eq!(reopened.active_profile_id(), None);
        assert_eq!(reopened.token(profile.id).unwrap(), None);

        let after = store(dir.path(), &tokens);
        assert!(after.profiles().is_empty());
    }

    #[test]
    fn the_file_is_private_and_never_holds_a_token() {
        let dir = tempfile::tempdir().unwrap();
        let tokens = Arc::new(MemoryTokenStore::new());
        let mut store = store(dir.path(), &tokens);
        store.add(profile(), "maine-SECRET-TOKEN-VALUE").unwrap();

        let path = dir.path().join(PROFILES_FILE_NAME);
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("SECRET-TOKEN-VALUE"));
        assert!(text.contains("serverURL"));
        // No temp file is left behind by the atomic write.
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n != PROFILES_FILE_NAME)
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn the_first_profile_is_active_and_later_ones_are_not() {
        let dir = tempfile::tempdir().unwrap();
        let tokens = Arc::new(MemoryTokenStore::new());
        let mut store = store(dir.path(), &tokens);
        let a = profile();
        let b = SyncProfile::new("Second", "https://example.com/maine");
        store.add(a.clone(), "a").unwrap();
        store.add(b.clone(), "b").unwrap();
        assert_eq!(store.active_profile_id(), Some(a.id));

        store.set_active(Some(b.id)).unwrap();
        assert_eq!(store.active_profile().map(|p| p.id), Some(b.id));
        assert!(store.set_active(Some(Uuid::new_v4())).is_err());
        assert_eq!(store.active_profile_id(), Some(b.id));

        // Removing the active profile leaves nothing active; the other stays.
        store.remove(b.id).unwrap();
        assert_eq!(store.active_profile_id(), None);
        assert_eq!(store.profiles().len(), 1);
        assert!(store.has_profiles());
    }

    #[test]
    fn update_persists_sync_state() {
        let dir = tempfile::tempdir().unwrap();
        let tokens = Arc::new(MemoryTokenStore::new());
        let mut store = store(dir.path(), &tokens);
        let p = profile();
        store.add(p.clone(), "t").unwrap();

        let changed = store
            .update(p.id, |p| {
                p.last_version = 7;
                p.last_sha256 = Some("abc".into());
                p.last_synced_at = Some(12.5);
                p.name = "Renamed".into();
            })
            .unwrap();
        assert!(changed);
        assert!(!store.update(Uuid::new_v4(), |_| {}).unwrap());

        let reopened = self::store(dir.path(), &tokens);
        let loaded = reopened.profile(p.id).unwrap();
        assert_eq!(loaded.last_version, 7);
        assert_eq!(loaded.last_sha256.as_deref(), Some("abc"));
        assert_eq!(loaded.last_synced_at, Some(12.5));
        assert_eq!(loaded.name, "Renamed");
    }

    #[test]
    fn duplicate_ids_are_refused_and_leave_the_token_alone() {
        let dir = tempfile::tempdir().unwrap();
        let tokens = Arc::new(MemoryTokenStore::new());
        let mut store = store(dir.path(), &tokens);
        let p = profile();
        store.add(p.clone(), "first").unwrap();
        assert!(store.add(p.clone(), "second").is_err());
        assert_eq!(store.token(p.id).unwrap().as_deref(), Some("first"));
        assert_eq!(store.profiles().len(), 1);
    }

    #[test]
    fn an_unreadable_file_is_kept_as_a_copy_and_the_list_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let tokens = Arc::new(MemoryTokenStore::new());
        let path = dir.path().join(PROFILES_FILE_NAME);
        fs::write(&path, b"{ this is not json").unwrap();

        let store = store(dir.path(), &tokens);
        assert!(store.profiles().is_empty());
        let backup = store.recovered_backup().expect("a backup name").to_string();
        assert!(backup.starts_with("sync-profiles.json.corrupt-"));
        assert_eq!(
            fs::read(dir.path().join(&backup)).unwrap(),
            b"{ this is not json"
        );
        // The original is untouched until something is saved.
        assert_eq!(fs::read(&path).unwrap(), b"{ this is not json");
    }

    #[test]
    fn an_active_id_without_a_profile_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let tokens = Arc::new(MemoryTokenStore::new());
        let path = dir.path().join(PROFILES_FILE_NAME);
        fs::write(
            &path,
            format!(
                r#"{{"version":1,"profiles":[],"activeProfile":"{}"}}"#,
                Uuid::new_v4()
            ),
        )
        .unwrap();
        let store = store(dir.path(), &tokens);
        assert_eq!(store.active_profile_id(), None);
    }

    #[test]
    fn a_missing_directory_is_created_with_private_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        let tokens = Arc::new(MemoryTokenStore::new());
        let mut store = SyncProfileStore::open(&nested, tokens).unwrap();
        store.add(profile(), "t").unwrap();
        let mode = fs::metadata(&nested).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }
}
