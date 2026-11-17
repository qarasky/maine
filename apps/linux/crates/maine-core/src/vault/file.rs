use super::contents::Contents;
use super::envelope::{context, derive_key, open, seal, VaultKey, ITERATIONS, MARKER};
use crate::VaultError;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// The on-disk vault envelope (JSON, binary fields base64).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Envelope {
    version: u32,
    #[serde(serialize_with = "crate::model::uuid_upper::serialize")]
    id: Uuid,
    #[serde(with = "b64")]
    salt: Vec<u8>,
    iterations: u32,
    #[serde(with = "b64")]
    verification: Vec<u8>,
    #[serde(default)]
    passwords: BTreeMap<String, String>,
    #[serde(default, with = "b64_opt", skip_serializing_if = "Option::is_none")]
    payload: Option<Vec<u8>>,
}

mod b64 {
    use super::*;
    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        STANDARD.decode(text).map_err(serde::de::Error::custom)
    }
}

mod b64_opt {
    use super::*;
    pub fn serialize<S: Serializer>(v: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(v) => s.serialize_str(&STANDARD.encode(v)),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        match Option::<String>::deserialize(d)? {
            Some(text) => STANDARD
                .decode(text)
                .map(Some)
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}

impl Envelope {
    pub(super) fn id(&self) -> Uuid {
        self.id
    }

    pub(super) fn context(&self) -> Vec<u8> {
        context(self.version, self.id, &self.salt, self.iterations)
    }
}

/// A vault key together with the vault it opens.
#[derive(Debug, Clone)]
pub struct Unlocked {
    pub id: Uuid,
    key: VaultKey,
}

impl Unlocked {
    pub fn key(&self) -> &VaultKey {
        &self.key
    }
}

/// Handle to one `passwords.maine` file. Holds only the path; every operation
/// re-reads the file, so a sync adopting a new copy is picked up automatically.
#[derive(Debug, Clone)]
pub struct VaultFile {
    path: PathBuf,
}

impl VaultFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `$XDG_DATA_HOME/maine/passwords.maine`, else `~/.local/share/maine/passwords.maine`.
    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("maine").join("passwords.maine")
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// Creates a new vault. Refuses to touch an existing file.
    pub fn create(&self, master: &str) -> Result<Unlocked, VaultError> {
        if master.chars().count() < 12 {
            return Err(VaultError::WeakPassword);
        }
        if self.exists() {
            return Err(VaultError::InvalidFile);
        }
        let mut salt = vec![0u8; 32];
        rand::thread_rng().fill_bytes(&mut salt);
        let key = derive_key(master, &salt, ITERATIONS);
        let mut env = Envelope {
            version: 2,
            id: Uuid::new_v4(),
            salt,
            iterations: ITERATIONS,
            verification: Vec::new(),
            passwords: BTreeMap::new(),
            payload: None,
        };
        let aad = env.context();
        env.verification = seal(MARKER, &key, &aad);
        env.payload = Some(seal(
            &serde_json::to_vec(&Contents::default()).map_err(|_| VaultError::InvalidFile)?,
            &key,
            &aad,
        ));
        self.write(&env)?;
        Ok(Unlocked { id: env.id, key })
    }

    /// Verifies the master password and returns the key. Never writes.
    pub fn unlock(&self, master: &str) -> Result<Unlocked, VaultError> {
        let env = self.read()?;
        let key = derive_key(master, &env.salt, env.iterations);
        Self::check_key(&env, &key)?;
        Self::decode_contents(&env, &key).map_err(|_| VaultError::InvalidPassword)?;
        Ok(Unlocked { id: env.id, key })
    }

    pub fn contents(&self, unlocked: &Unlocked) -> Result<Contents, VaultError> {
        let env = self.read()?;
        if env.id != unlocked.id {
            return Err(VaultError::Locked);
        }
        Self::check_key(&env, &unlocked.key).map_err(|_| VaultError::InvalidFile)?;
        Self::decode_contents(&env, &unlocked.key)
    }

    /// Applies `f` to the contents and writes the vault back atomically.
    pub fn update(
        &self,
        unlocked: &Unlocked,
        f: impl FnOnce(&mut Contents),
    ) -> Result<(), VaultError> {
        let mut env = self.read()?;
        if env.id != unlocked.id {
            return Err(VaultError::Locked);
        }
        Self::check_key(&env, &unlocked.key).map_err(|_| VaultError::InvalidFile)?;
        let mut contents = Self::decode_contents(&env, &unlocked.key)?;
        f(&mut contents);
        contents.sessions = None;
        let plain = serde_json::to_vec(&contents).map_err(|_| VaultError::InvalidFile)?;
        env.payload = Some(seal(&plain, &unlocked.key, &env.context()));
        self.write(&env)
    }

    /// Checks `data` as a full vault under `master` without touching any file.
    pub fn validate_blob(data: &[u8], master: &str) -> Result<(), VaultError> {
        let env = Self::decode(data)?;
        let key = derive_key(master, &env.salt, env.iterations);
        Self::check_key(&env, &key)?;
        Self::decode_contents(&env, &key).map_err(|_| VaultError::InvalidFile)?;
        Ok(())
    }

    /// True when `unlocked`'s key opens `data` as a version of this same vault.
    pub fn key_opens(data: &[u8], unlocked: &Unlocked) -> bool {
        match Self::decode(data) {
            Ok(env) => env.id == unlocked.id && Self::check_key(&env, &unlocked.key).is_ok(),
            Err(_) => false,
        }
    }

    // MARK: internals

    pub(super) fn check_key(env: &Envelope, key: &VaultKey) -> Result<(), VaultError> {
        let marker = open(&env.verification, key, &env.context())
            .map_err(|_| VaultError::InvalidPassword)?;
        if marker == MARKER {
            Ok(())
        } else {
            Err(VaultError::InvalidPassword)
        }
    }

    fn decode_contents(env: &Envelope, key: &VaultKey) -> Result<Contents, VaultError> {
        let payload = env.payload.as_deref().ok_or(VaultError::InvalidFile)?;
        let plain = open(payload, key, &env.context())?;
        serde_json::from_slice(&plain).map_err(|_| VaultError::InvalidFile)
    }

    pub(super) fn read(&self) -> Result<Envelope, VaultError> {
        let data = fs::read(&self.path)?;
        Self::decode(&data)
    }

    /// Structural validation shared by every entry point. Never touches disk.
    fn decode(data: &[u8]) -> Result<Envelope, VaultError> {
        let env: Envelope = serde_json::from_slice(data).map_err(|_| VaultError::InvalidFile)?;
        if env.version == 1 {
            return Err(VaultError::UnsupportedVersion(1));
        }
        let ok = env.version == 2
            && env.salt.len() == 32
            && env.iterations == ITERATIONS
            && env.payload.is_some()
            && env.passwords.is_empty();
        if ok {
            Ok(env)
        } else {
            Err(VaultError::InvalidFile)
        }
    }

    fn write(&self, env: &Envelope) -> Result<(), VaultError> {
        let bytes = serde_json::to_vec_pretty(env).map_err(|_| VaultError::InvalidFile)?;
        atomic_write(&self.path, &bytes)
    }
}

/// Temp file in the same directory, fsync, rename, so a crash leaves either
/// the old or the new file, never half of one. File mode 0600, new directories 0700.
pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), VaultError> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if !dir.exists() {
        fs::create_dir_all(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    let mut suffix = [0u8; 6];
    rand::thread_rng().fill_bytes(&mut suffix);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("vault");
    let tmp = dir.join(format!(
        ".{name}.tmp-{}",
        suffix
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        File::open(dir)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    Ok(result?)
}
