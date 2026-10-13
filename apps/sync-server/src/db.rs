//! Embedded SQLite storage.
//!
//! Two tables: one row per slot (token hash and current version) and one row
//! per stored version (the opaque blob). Blobs are never parsed, inspected or
//! transformed — the server is a vault-shaped key-value store.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};

use crate::hash;

/// A slot as the API needs it: identity, current version and metadata.
#[derive(Clone, Copy, Debug)]
pub struct Slot {
    pub id: i64,
    pub version: i64,
    pub size: i64,
    pub updated_at: Option<i64>,
}

/// What a write expects to find.
#[derive(Clone, Copy, Debug)]
pub enum Expected {
    /// The slot must have no stored version yet.
    Empty,
    /// The slot's current version must be exactly this one.
    Version(i64),
}

/// Outcome of a conditional write. Conflicts are values, not errors, because
/// both are normal API responses.
#[derive(Debug)]
pub enum PutOutcome {
    Stored {
        version: i64,
        sha256: [u8; 32],
    },
    /// Another writer moved the slot first.
    Conflict(Slot),
    /// The writer expected an empty slot, but it is not empty.
    NotEmpty(Slot),
}

#[derive(Clone, Debug)]
pub struct VersionMeta {
    pub version: i64,
    pub size: i64,
    pub sha256: [u8; 32],
    pub created_at: i64,
}

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating data directory {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("opening database {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;

             CREATE TABLE IF NOT EXISTS slots (
                 id         INTEGER PRIMARY KEY,
                 token_hash BLOB    NOT NULL UNIQUE,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER,
                 version    INTEGER NOT NULL DEFAULT 0,
                 size       INTEGER NOT NULL DEFAULT 0
             );

             CREATE TABLE IF NOT EXISTS versions (
                 slot_id    INTEGER NOT NULL REFERENCES slots(id) ON DELETE CASCADE,
                 version    INTEGER NOT NULL,
                 size       INTEGER NOT NULL,
                 sha256     BLOB    NOT NULL,
                 created_at INTEGER NOT NULL,
                 blob       BLOB    NOT NULL,
                 PRIMARY KEY (slot_id, version)
             ) WITHOUT ROWID;",
        )
        .context("initializing the database schema")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> Result<MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))
    }

    /// Create a slot, optionally with its first stored version.
    pub fn create_slot(
        &self,
        token_hash: &[u8; 32],
        blob: Option<&[u8]>,
    ) -> Result<(Slot, Option<[u8; 32]>)> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let now = now();
        tx.execute(
            "INSERT INTO slots (token_hash, created_at) VALUES (?1, ?2)",
            params![&token_hash[..], now],
        )?;
        let id = tx.last_insert_rowid();
        let stored = if let Some(blob) = blob {
            let sha256 = hash::sha256(blob);
            tx.execute(
                "INSERT INTO versions (slot_id, version, size, sha256, created_at, blob)
                 VALUES (?1, 1, ?2, ?3, ?4, ?5)",
                params![id, blob.len() as i64, &sha256[..], now, blob],
            )?;
            tx.execute(
                "UPDATE slots SET version = 1, size = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, blob.len() as i64, now],
            )?;
            (
                Slot {
                    id,
                    version: 1,
                    size: blob.len() as i64,
                    updated_at: Some(now),
                },
                Some(sha256),
            )
        } else {
            (
                Slot {
                    id,
                    version: 0,
                    size: 0,
                    updated_at: None,
                },
                None,
            )
        };
        tx.commit()?;
        Ok(stored)
    }

    pub fn find_slot(&self, token_hash: &[u8; 32]) -> Result<Option<Slot>> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT id, version, size, updated_at FROM slots WHERE token_hash = ?1",
            params![&token_hash[..]],
            row_to_slot,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Store a new version under a version precondition, then prune history.
    pub fn put(
        &self,
        slot_id: i64,
        expected: Expected,
        blob: &[u8],
        keep_versions: u32,
    ) -> Result<PutOutcome> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let slot = tx.query_row(
            "SELECT id, version, size, updated_at FROM slots WHERE id = ?1",
            params![slot_id],
            row_to_slot,
        )?;
        match expected {
            Expected::Empty if slot.version != 0 => return Ok(PutOutcome::NotEmpty(slot)),
            Expected::Version(version) if slot.version != version => {
                return Ok(PutOutcome::Conflict(slot));
            }
            _ => {}
        }

        let now = now();
        let sha256 = hash::sha256(blob);
        let version = slot.version + 1;
        tx.execute(
            "INSERT INTO versions (slot_id, version, size, sha256, created_at, blob)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![slot_id, version, blob.len() as i64, &sha256[..], now, blob],
        )?;
        tx.execute(
            "UPDATE slots SET version = ?2, size = ?3, updated_at = ?4 WHERE id = ?1",
            params![slot_id, version, blob.len() as i64, now],
        )?;
        let cutoff = version - i64::from(keep_versions.max(1));
        tx.execute(
            "DELETE FROM versions WHERE slot_id = ?1 AND version <= ?2",
            params![slot_id, cutoff],
        )?;
        tx.commit()?;
        Ok(PutOutcome::Stored { version, sha256 })
    }

    /// Point the slot at a new token hash. The old token stops working
    /// immediately; there is no grace period.
    pub fn rotate(&self, slot_id: i64, token_hash: &[u8; 32]) -> Result<()> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE slots SET token_hash = ?2 WHERE id = ?1",
            params![slot_id, &token_hash[..]],
        )?;
        if changed == 0 {
            bail!("slot {slot_id} disappeared");
        }
        Ok(())
    }

    /// Remove a slot and every stored version.
    pub fn delete(&self, slot_id: i64) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM slots WHERE id = ?1", params![slot_id])?;
        Ok(())
    }

    /// Metadata of stored versions, newest first.
    pub fn versions(&self, slot_id: i64) -> Result<Vec<VersionMeta>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT version, size, sha256, created_at FROM versions
             WHERE slot_id = ?1 ORDER BY version DESC",
        )?;
        let rows = stmt.query_map(params![slot_id], |row| {
            let sha: Vec<u8> = row.get(2)?;
            let mut sha256 = [0u8; 32];
            if sha.len() == 32 {
                sha256.copy_from_slice(&sha);
            }
            Ok(VersionMeta {
                version: row.get(0)?,
                size: row.get(1)?,
                sha256,
                created_at: row.get(3)?,
            })
        })?;
        let mut versions = Vec::new();
        for row in rows {
            versions.push(row?);
        }
        Ok(versions)
    }

    /// The blob stored for one specific version, if it has not been pruned.
    pub fn blob(&self, slot_id: i64, version: i64) -> Result<Option<Vec<u8>>> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT blob FROM versions WHERE slot_id = ?1 AND version = ?2",
            params![slot_id, version],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }
}

fn row_to_slot(row: &rusqlite::Row<'_>) -> rusqlite::Result<Slot> {
    Ok(Slot {
        id: row.get(0)?,
        version: row.get(1)?,
        size: row.get(2)?,
        updated_at: row.get(3)?,
    })
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}
