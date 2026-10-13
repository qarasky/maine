//! Command-line configuration. Every option has an environment equivalent so
//! containers and service managers never need a shell wrapper.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;

/// Loopback by default: exposing the server is a deliberate choice.
pub const DEFAULT_BIND: &str = "127.0.0.1:8484";

#[derive(Parser, Debug)]
#[command(
    name = "maine-sync-server",
    version,
    about = "Zero-knowledge sync server for the Maine SSH client"
)]
pub struct Cli {
    /// Address to listen on, for example 0.0.0.0:8484 behind a reverse proxy.
    #[arg(long, env = "MAINE_SYNC_BIND", default_value = DEFAULT_BIND)]
    pub bind: SocketAddr,

    /// Directory for the embedded SQLite database.
    #[arg(long, env = "MAINE_SYNC_DATA_DIR", default_value = "./data")]
    pub data_dir: PathBuf,

    /// How many recent versions of each vault to keep for recovery.
    #[arg(
        long,
        env = "MAINE_SYNC_KEEP_VERSIONS",
        default_value_t = 32,
        value_parser = clap::value_parser!(u32).range(1..)
    )]
    pub keep_versions: u32,

    /// Largest accepted vault: 64MiB, 512KiB, or a plain byte count.
    #[arg(
        long,
        env = "MAINE_SYNC_MAX_BLOB",
        default_value = "64MiB",
        value_parser = parse_size
    )]
    pub max_blob: u64,

    /// Optional secret required to create slots, sent in the
    /// `X-Maine-Create-Secret` header. Set it whenever the server is reachable
    /// by anyone you do not trust to create slots.
    #[arg(long, env = "MAINE_SYNC_CREATE_SECRET")]
    pub create_secret: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub keep_versions: u32,
    pub max_blob: u64,
    pub create_secret: Option<String>,
}

impl From<Cli> for Config {
    fn from(cli: Cli) -> Self {
        Self {
            bind: cli.bind,
            data_dir: cli.data_dir,
            keep_versions: cli.keep_versions,
            max_blob: cli.max_blob,
            create_secret: cli.create_secret,
        }
    }
}

impl Config {
    /// Database file inside the data directory.
    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("sync.sqlite3")
    }
}

/// Parse `64MiB`, `512KiB`, `4096` and friends. Deliberately rejects ambiguous
/// decimal units like `MB`; binary suffixes are the only ones accepted.
pub fn parse_size(raw: &str) -> Result<u64, String> {
    let trimmed = raw.trim();
    let split = trimmed
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let (digits, suffix) = trimmed.split_at(split);
    if digits.is_empty() {
        return Err(format!("expected a size like 64MiB, got {raw:?}"));
    }
    let value: u64 = digits.parse().map_err(|_| format!("bad size {raw:?}"))?;
    let multiplier: u64 = match suffix.trim().to_ascii_uppercase().as_str() {
        "" | "B" => 1,
        "K" | "KIB" => 1024,
        "M" | "MIB" => 1024 * 1024,
        "G" | "GIB" => 1024 * 1024 * 1024,
        other => {
            return Err(format!(
                "unknown size suffix {other:?}; use B, KiB, MiB or GiB"
            ));
        }
    };
    value
        .checked_mul(multiplier)
        .filter(|size| *size > 0)
        .ok_or_else(|| format!("size {raw:?} is out of range"))
}

#[cfg(test)]
mod tests {
    use super::parse_size;

    #[test]
    fn parses_binary_suffixes() {
        assert_eq!(parse_size("4096").unwrap(), 4096);
        assert_eq!(parse_size("1B").unwrap(), 1);
        assert_eq!(parse_size("2K").unwrap(), 2 * 1024);
        assert_eq!(parse_size("64MiB").unwrap(), 64 * 1024 * 1024);
        assert_eq!(parse_size("1 GiB").unwrap(), 1024 * 1024 * 1024);
    }

    #[test]
    fn rejects_ambiguous_or_invalid_sizes() {
        assert!(parse_size("").is_err());
        assert!(parse_size("MiB").is_err());
        assert!(parse_size("10MB").is_err());
        assert!(parse_size("0").is_err());
        assert!(parse_size("-1").is_err());
        assert!(parse_size("99999999999999999999").is_err());
    }
}
