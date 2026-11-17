//! Device-local state: saved sessions, collapsed groups and agent data. Lives
//! beside the vault in `sessions-<vault id>.maine`, sealed with the vault key
//! and bound to the vault's context, and is never synced.

use super::contents::SavedSession;
use super::envelope::{open, seal};
use super::file::{atomic_write, Envelope, Unlocked, VaultFile};
use crate::model::uuid_upper::upper;
use crate::VaultError;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

/// Everything this device remembers about a vault that must not sync.
/// `agent_access` and `agent_audit` are owned by the agents crate and kept
/// opaque here so unknown fields survive a round trip.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LocalState {
    #[serde(default)]
    pub sessions: Vec<SavedSession>,
    #[serde(
        default,
        rename = "collapsedGroups",
        serialize_with = "serialize_uuids"
    )]
    pub collapsed_groups: Vec<Uuid>,
    #[serde(
        default,
        rename = "agentAccess",
        skip_serializing_if = "Option::is_none"
    )]
    pub agent_access: Option<Value>,
    #[serde(default, rename = "agentAudit")]
    pub agent_audit: Vec<Value>,
}

fn serialize_uuids<S: serde::Serializer>(ids: &[Uuid], s: S) -> Result<S::Ok, S::Error> {
    s.collect_seq(ids.iter().map(upper))
}

#[derive(Serialize, Deserialize)]
struct SessionsFile {
    /// 1: payload is a bare `[SavedSession]`. 2: payload is `LocalState`.
    version: u32,
    #[serde(serialize_with = "crate::model::uuid_upper::serialize")]
    vault: Uuid,
    payload: String,
}

fn aad(env: &Envelope) -> Vec<u8> {
    let mut context = env.context();
    context.extend_from_slice(b":sessions");
    context
}

impl VaultFile {
    /// `sessions-<VAULT-ID>.maine`, next to the vault file.
    pub fn local_state_path(&self, vault_id: Uuid) -> PathBuf {
        self.path()
            .with_file_name(format!("sessions-{}.maine", upper(&vault_id)))
    }

    /// A missing file, a file for another vault or one that does not open with
    /// this key reads as empty and is replaced by the next write.
    pub fn local_state(&self, unlocked: &Unlocked) -> Result<LocalState, VaultError> {
        let env = self.read()?;
        if env.id() != unlocked.id {
            return Err(VaultError::Locked);
        }
        Self::check_key(&env, unlocked.key()).map_err(|_| VaultError::InvalidFile)?;
        let Ok(data) = std::fs::read(self.local_state_path(unlocked.id)) else {
            return Ok(LocalState::default());
        };
        let Ok(file) = serde_json::from_slice::<SessionsFile>(&data) else {
            return Ok(LocalState::default());
        };
        if file.vault != unlocked.id {
            return Ok(LocalState::default());
        }
        let Ok(sealed) = STANDARD.decode(&file.payload) else {
            return Ok(LocalState::default());
        };
        let Ok(plain) = open(&sealed, unlocked.key(), &aad(&env)) else {
            return Ok(LocalState::default());
        };
        Ok(match file.version {
            1 => LocalState {
                sessions: serde_json::from_slice(&plain).unwrap_or_default(),
                ..Default::default()
            },
            2 => serde_json::from_slice(&plain).unwrap_or_default(),
            _ => LocalState::default(),
        })
    }

    /// Applies `f`; writes only when something changed. Deliberately silent:
    /// sync watches vault writes only, and this file is never pushed.
    pub fn update_local_state(
        &self,
        unlocked: &Unlocked,
        f: impl FnOnce(&mut LocalState),
    ) -> Result<(), VaultError> {
        let env = self.read()?;
        let mut state = self.local_state(unlocked)?;
        let before = state.clone();
        f(&mut state);
        if state == before {
            return Ok(());
        }
        let plain = serde_json::to_vec(&state).map_err(|_| VaultError::InvalidFile)?;
        let file = SessionsFile {
            version: 2,
            vault: unlocked.id,
            payload: STANDARD.encode(seal(&plain, unlocked.key(), &aad(&env))),
        };
        let bytes = serde_json::to_vec_pretty(&file).map_err(|_| VaultError::InvalidFile)?;
        atomic_write(&self.local_state_path(unlocked.id), &bytes)
    }
}
