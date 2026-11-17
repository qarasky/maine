//! Glue between `maine-sync` and the engine: the `VaultHost` the sync
//! coordinator calls back into, and the global handle the bridge drives.

use crate::secrets::SyncTokens;
use maine_core::vault::Unlocked;
use maine_sync::{AdoptReport, SyncHandle, SyncProfileStore, SyncStatus, VaultHost};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use crate::state::with_engine;

static HANDLE: OnceLock<SyncHandle> = OnceLock::new();
static ALERT: Mutex<Option<(String, String)>> = Mutex::new(None);

struct AppHost;

impl VaultHost for AppHost {
    fn vault_path(&self) -> PathBuf {
        with_engine(|e| e.paths().vault.clone())
    }

    fn unlocked(&self) -> Option<Unlocked> {
        with_engine(|e| e.unlocked().cloned())
    }

    fn on_remote_adopted(&self, _report: &AdoptReport) {
        // Reload hosts and groups, close sessions whose host is gone; surviving terminals are never touched.
        with_engine(|e| {
            let _ = e.adopt_remote_change();
        });
        crate::bridge::request_refresh();
    }

    fn on_status(&self, _status: &SyncStatus) {
        crate::bridge::request_refresh();
    }

    fn on_alert(&self, title: &str, message: &str) {
        *ALERT.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((title.to_string(), message.to_string()));
        crate::bridge::request_refresh();
    }

    fn detach_all_for_vault_switch(&self) -> Result<(), String> {
        with_engine(|e| e.sessions_mut().detach_all());
        Ok(())
    }

    fn on_workspace_switched(&self) {
        with_engine(|e| e.lock());
        crate::bridge::request_refresh();
    }
}

/// Starts the coordinator. Profiles are read from `config_dir`; tokens live in the Secret Service.
pub fn init(config_dir: PathBuf) -> Result<(), String> {
    if HANDLE.get().is_some() {
        return Ok(());
    }
    let store =
        SyncProfileStore::open(config_dir, Arc::new(SyncTokens)).map_err(|e| e.to_string())?;
    let handle = SyncHandle::start_default(store, Arc::new(AppHost));
    let _ = HANDLE.set(handle);
    Ok(())
}

pub fn handle() -> Option<&'static SyncHandle> {
    HANDLE.get()
}

pub fn notify_local_write() {
    if let Some(h) = handle() {
        h.notify_local_write();
    }
}

pub fn unlocked() {
    if let Some(h) = handle() {
        h.unlocked();
    }
}

pub fn locked() {
    if let Some(h) = handle() {
        h.locked();
    }
}

pub fn foreground() {
    if let Some(h) = handle() {
        h.foreground();
    }
}

/// Best-effort push before the process ends (bounded to 2 seconds).
pub fn flush_before_quit() {
    if let Some(h) = handle() {
        if h.wants_flush_before_quit() {
            h.quit_flush(std::time::Duration::from_secs(2));
        }
    }
}

/// The alert to show once (conflict banner, "Sync stopped").
pub fn take_alert() -> Option<(String, String)> {
    ALERT.lock().unwrap_or_else(|e| e.into_inner()).take()
}

pub fn status_kind(status: &SyncStatus) -> &'static str {
    match status {
        SyncStatus::Off => "off",
        SyncStatus::Idle => "idle",
        SyncStatus::Syncing => "syncing",
        SyncStatus::Offline(_) => "offline",
        SyncStatus::TokenRejected => "tokenRejected",
        SyncStatus::Conflicted { .. } => "conflicted",
        SyncStatus::Failed(_) => "failed",
    }
}

/// Sync state for the Settings page and the sidebar footer.
pub fn json() -> serde_json::Value {
    use serde_json::json;
    let Some(h) = handle() else {
        return json!({ "ready": false, "profiles": [], "activeId": null, "status": "off", "statusLabel": "Sync off" });
    };
    let status = h.status();
    let active = h.active_profile().map(|p| p.id.to_string());
    json!({
        "ready": true,
        "profiles": h.profiles().iter().map(|p| json!({
            "id": p.id.to_string(),
            "name": p.name,
            "serverURL": p.server_url,
            "lastVersion": p.last_version,
            // Seconds since 2001-01-01 -> Unix seconds for QML's Date.
            "lastSyncedAt": p.last_synced_at.map(|s| s + 978_307_200.0),
        })).collect::<Vec<_>>(),
        "activeId": active,
        "status": status_kind(&status),
        "statusLabel": status.label(),
    })
}
