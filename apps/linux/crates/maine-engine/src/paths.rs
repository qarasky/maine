use maine_core::vault::VaultFile;
use std::path::{Path, PathBuf};

/// Where Maine keeps things on Linux (XDG).
#[derive(Debug, Clone)]
pub struct Paths {
    /// `passwords.maine`
    pub vault: PathBuf,
    /// 0700 directory for sockets: `$XDG_RUNTIME_DIR/maine`
    pub runtime_dir: PathBuf,
    /// Non-secret configuration: `$XDG_CONFIG_HOME/maine`
    pub config_dir: PathBuf,
}

impl Paths {
    pub fn from_env() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(format!("/tmp/maine-{}", rustix::process::getuid().as_raw()))
            });
        Self {
            vault: VaultFile::default_path(),
            runtime_dir: runtime.join("maine"),
            config_dir: config.join("maine"),
        }
    }

    /// Everything under one directory, for tests.
    pub fn under(dir: &Path) -> Self {
        Self {
            vault: dir.join("data").join("passwords.maine"),
            runtime_dir: dir.join("run"),
            config_dir: dir.join("config"),
        }
    }

    pub fn askpass_socket(&self) -> PathBuf {
        self.runtime_dir.join("askpass.sock")
    }

    pub fn agent_socket(&self) -> PathBuf {
        self.runtime_dir.join("agent.sock")
    }
}
