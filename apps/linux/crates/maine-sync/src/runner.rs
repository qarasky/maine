//! The thread around [`Coordinator`] and the handle the app holds.
//!
//! The notifications (`notify_local_write`, `unlocked`, `locked`,
//! `foreground`) never block: they enqueue a message for the runner thread.
//! `status()` reads a snapshot and never waits for a request in flight. The
//! pairing and workspace operations run on the caller's thread under the
//! coordinator lock, so call them from a worker thread, not the UI thread;
//! they wait for any request already in flight (a few seconds at most).

use crate::coordinator::{
    lock, Coordinator, CoordinatorConfig, CreatedProfile, JoinOutcome, Shared, SharedProfiles,
    SyncStatus, VaultHost,
};
use crate::engine::sha256_hex;
use crate::error::SyncError;
use crate::profile::{SyncProfile, SyncProfileStore};
use crate::time::system_clock;
use crate::transport::HttpTransport;
use std::fs;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use uuid::Uuid;

/// The longest the runner sleeps without re-reading the schedule.
const MAX_IDLE: Duration = Duration::from_secs(60);

enum Message {
    LocalWrite,
    Unlocked,
    Locked,
    Foreground,
    Flush {
        timeout: Duration,
        done: Sender<()>,
    },
    /// The schedule may have changed (a pairing operation ran).
    Wake,
    Shutdown,
}

pub struct SyncHandle {
    coordinator: Arc<Mutex<Coordinator>>,
    sender: Sender<Message>,
    shared: Arc<Shared>,
    profiles: SharedProfiles,
    host: Arc<dyn VaultHost>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl SyncHandle {
    /// Starts the runner thread around a coordinator.
    pub fn start(coordinator: Coordinator) -> Self {
        let shared = coordinator.shared();
        let profiles = coordinator.profiles_handle();
        let host = coordinator.host_handle();
        let coordinator = Arc::new(Mutex::new(coordinator));
        let (sender, receiver) = mpsc::channel();
        let runner = coordinator.clone();
        let thread = thread::Builder::new()
            .name("maine-sync".into())
            .spawn(move || run(&runner, &receiver))
            .expect("the sync thread must start");
        Self {
            coordinator,
            sender,
            shared,
            profiles,
            host,
            thread: Mutex::new(Some(thread)),
        }
    }

    /// The production wiring: real HTTP, the system clock, default timings.
    pub fn start_default(profiles: SyncProfileStore, host: Arc<dyn VaultHost>) -> Self {
        Self::start(Coordinator::new(
            Arc::new(HttpTransport::new()),
            Arc::new(Mutex::new(profiles)),
            host,
            system_clock(),
            CoordinatorConfig::default(),
        ))
    }

    // MARK: - Notifications (never block)

    /// The vault file was written locally; a push follows after the debounce.
    pub fn notify_local_write(&self) {
        let _ = self.sender.send(Message::LocalWrite);
    }

    /// The vault just unlocked: pull and start polling.
    pub fn unlocked(&self) {
        let _ = self.sender.send(Message::Unlocked);
    }

    /// The vault is about to lock: best-effort push, then stop polling.
    pub fn locked(&self) {
        let _ = self.sender.send(Message::Locked);
    }

    /// The app window regained focus.
    pub fn foreground(&self) {
        let _ = self.sender.send(Message::Foreground);
    }

    /// Waits up to `timeout` for the last push before quitting. Returns
    /// whether it finished in time; the request itself is bounded by the same
    /// timeout. Does nothing (and returns `true`) when there is nothing to push.
    pub fn quit_flush(&self, timeout: Duration) -> bool {
        let (done, wait) = mpsc::channel();
        if self.sender.send(Message::Flush { timeout, done }).is_err() {
            return false;
        }
        wait.recv_timeout(timeout + Duration::from_millis(250))
            .is_ok()
    }

    // MARK: - Reads (never wait for the network)

    pub fn status(&self) -> SyncStatus {
        self.shared.status()
    }

    pub fn is_unlocked(&self) -> bool {
        self.shared.is_unlocked()
    }

    /// True when quitting should call `quit_flush`: unlocked, a profile is
    /// active and the vault file differs from the last synced bytes.
    pub fn wants_flush_before_quit(&self) -> bool {
        if !self.shared.is_unlocked() {
            return false;
        }
        let Some(profile) = lock(&self.profiles).active_profile().cloned() else {
            return false;
        };
        let current = fs::read(self.host.vault_path())
            .ok()
            .map(|bytes| sha256_hex(&bytes));
        current != profile.last_sha256
    }

    pub fn profiles(&self) -> Vec<SyncProfile> {
        lock(&self.profiles).profiles().to_vec()
    }

    pub fn active_profile(&self) -> Option<SyncProfile> {
        lock(&self.profiles).active_profile().cloned()
    }

    pub fn rename_profile(&self, id: Uuid, name: &str) -> Result<bool, SyncError> {
        let name = name.trim();
        if name.is_empty() {
            return Ok(false);
        }
        lock(&self.profiles).update(id, |p| p.name = name.to_string())
    }

    // MARK: - Pairing and workspaces (blocking; call from a worker thread)

    fn with<R>(&self, operation: impl FnOnce(&mut Coordinator) -> R) -> R {
        let result = operation(&mut lock(&self.coordinator));
        let _ = self.sender.send(Message::Wake);
        result
    }

    pub fn create_profile(
        &self,
        server: &str,
        name: &str,
        create_secret: Option<&str>,
    ) -> Result<CreatedProfile, SyncError> {
        self.with(|c| c.create_profile(server, name, create_secret))
    }

    pub fn join_profile(
        &self,
        server: &str,
        token: &str,
        name: &str,
        master_password: &str,
    ) -> Result<JoinOutcome, SyncError> {
        self.with(|c| c.join_profile(server, token, name, master_password))
    }

    pub fn push_local_vault_into_empty_slot(
        &self,
        server: &str,
        token: &str,
        name: &str,
    ) -> Result<SyncProfile, SyncError> {
        self.with(|c| c.push_local_vault_into_empty_slot(server, token, name))
    }

    pub fn rotate_token(&self, id: Uuid) -> Result<String, SyncError> {
        self.with(|c| c.rotate_token(id))
    }

    pub fn disable(&self, id: Uuid) -> Result<(), SyncError> {
        self.with(|c| c.disable(id))
    }

    pub fn delete_remote_data(&self, id: Uuid) -> Result<(), SyncError> {
        self.with(|c| c.delete_remote_data(id))
    }

    pub fn activate(&self, id: Uuid) -> Result<(), SyncError> {
        self.with(|c| c.activate(id))
    }

    /// Stops the runner thread and waits for it. Also done on drop.
    pub fn shutdown(&self) {
        let _ = self.sender.send(Message::Shutdown);
        if let Some(thread) = lock(&self.thread).take() {
            let _ = thread.join();
        }
    }
}

impl Drop for SyncHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run(coordinator: &Mutex<Coordinator>, receiver: &mpsc::Receiver<Message>) {
    loop {
        let wait = lock(coordinator).wait_hint();
        let received = match wait {
            Some(wait) => receiver.recv_timeout(wait.min(MAX_IDLE)),
            None => receiver.recv_timeout(MAX_IDLE),
        };
        match received {
            Err(RecvTimeoutError::Disconnected) | Ok(Message::Shutdown) => return,
            Err(RecvTimeoutError::Timeout) => lock(coordinator).tick(),
            Ok(message) => {
                let mut coordinator = lock(coordinator);
                match message {
                    Message::LocalWrite => coordinator.local_vault_changed(),
                    Message::Unlocked => coordinator.vault_did_unlock(),
                    Message::Locked => coordinator.vault_will_lock(),
                    Message::Foreground => coordinator.foreground(),
                    Message::Flush { timeout, done } => {
                        coordinator.flush_before_quit(timeout);
                        let _ = done.send(());
                    }
                    Message::Wake | Message::Shutdown => {}
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
