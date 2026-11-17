//! When to sync, as a state machine with no threads and no sleeping.
//!
//! [`Coordinator`] is the port of macOS `SyncCoordinator`: the triggers (pull
//! on unlock, a 2 s write debounce, a 30 s `HEAD` poll with 30 s / 2 min /
//! 5 min backoff, a best-effort push on lock, a bounded flush on quit), the
//! status vocabulary, and the pairing and workspace operations. Every method
//! runs to completion on the calling thread and reads time from an injected
//! [`Clock`], so the whole thing is tested by moving a manual clock. The
//! runner thread in `runner.rs` merely calls `tick()` when a deadline is due.
//!
//! Nothing here knows about a UI. The app implements [`VaultHost`]; callbacks
//! arrive on whichever thread ran the operation, with the coordinator locked,
//! so they must hand work to the UI thread and must not call back into the
//! sync handle synchronously (except the non-blocking notifications).

use crate::engine::{JoinPreparation, PullResult, PushResult, SyncEngine};
use crate::error::SyncError;
use crate::profile::{SyncProfile, SyncProfileStore};
use crate::time::{unix_to_apple, Clock};
use crate::transport::{FetchResult, SlotState, Transport};
use crate::url::{host_of, normalize_server_url};
use maine_core::vault::{Contents, Unlocked, VaultFile};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The profile list shared between the coordinator (which updates sync state
/// after every operation) and the app (which lists and renames profiles). The
/// lock is only ever held for a quick read or write, never across a request.
pub type SharedProfiles = Arc<Mutex<SyncProfileStore>>;

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// MARK: - Status

/// What the sync dot and label show. Wording is the macOS vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStatus {
    Off,
    Idle,
    Syncing,
    Offline(String),
    TokenRejected,
    /// A dirty local vault was saved as this file name before adopting the
    /// server's copy.
    Conflicted {
        copy: String,
    },
    Failed(String),
}

impl SyncStatus {
    pub fn label(&self) -> String {
        match self {
            Self::Off => "Sync off".into(),
            Self::Idle => "Synced".into(),
            Self::Syncing => "Syncing\u{2026}".into(),
            Self::Offline(message) | Self::Failed(message) => message.clone(),
            Self::TokenRejected => "Token rejected".into(),
            Self::Conflicted { copy } => format!("Conflict saved as {copy}"),
        }
    }
}

/// What changed when a remote vault replaced the live file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptReport {
    /// The server version now on disk.
    pub version: u64,
    /// The dirty local vault, saved beside the vault, when there was one.
    pub conflict_copy: Option<PathBuf>,
    pub conflict_copy_name: Option<String>,
    /// True when the adopted vault's hosts, groups or saved passwords differ
    /// from the vault it replaced (or the old one could not be read). The app
    /// reloads its host list when this is set.
    pub hosts_changed: bool,
    /// Hosts the old vault had and the adopted one does not. Sessions on these
    /// must close; every other session keeps running untouched.
    pub removed_host_ids: Vec<Uuid>,
}

impl AdoptReport {
    /// The alert macOS shows for a conflict: `(title, message)`.
    pub fn banner(&self) -> Option<(String, String)> {
        self.conflict_copy_name.as_ref().map(|copy| {
            (
                "Another device changed the vault".to_string(),
                format!(
                    "Your version was saved as {copy}. This device now follows the server's copy."
                ),
            )
        })
    }
}

/// What the coordinator needs from the app.
pub trait VaultHost: Send + Sync {
    /// The live `passwords.maine`. Read once when the coordinator is built.
    fn vault_path(&self) -> PathBuf;
    /// The key of the unlocked vault, `None` while locked. Used only to check
    /// that a pulled vault is a version of this same vault.
    fn unlocked(&self) -> Option<Unlocked>;
    /// The live file was replaced by the server's copy. Reload hosts, close
    /// sessions in `removed_host_ids`, restore saved sessions; never reparent
    /// or restart a surviving terminal.
    fn on_remote_adopted(&self, report: &AdoptReport);
    /// The status changed (never called twice in a row with the same value).
    fn on_status(&self, status: &SyncStatus);
    /// A message the user must see (`title`, `message`): the conflict banner
    /// and "Sync stopped".
    fn on_alert(&self, _title: &str, _message: &str) {}
    /// Called before a vault switch (join with another profile active,
    /// activate). Checkpoint and detach every open session; an `Err` aborts
    /// the switch with that message and changes nothing.
    fn detach_all_for_vault_switch(&self) -> Result<(), String> {
        Ok(())
    }
    /// A workspace switch finished: reload, lock the vault and ask for the new
    /// vault's master password.
    fn on_workspace_switched(&self) {}
}

/// Timing knobs. The defaults are the documented values.
#[derive(Debug, Clone)]
pub struct CoordinatorConfig {
    pub debounce: Duration,
    pub poll_interval: Duration,
    /// Poll delays after consecutive failures; the last repeats.
    pub backoff: Vec<Duration>,
    /// The bound on the push before a workspace switch.
    pub switch_push_timeout: Duration,
}

impl Default for CoordinatorConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_secs(2),
            poll_interval: Duration::from_secs(30),
            backoff: vec![
                Duration::from_secs(30),
                Duration::from_secs(120),
                Duration::from_secs(300),
            ],
            switch_push_timeout: Duration::from_secs(3),
        }
    }
}

/// State readable from any thread without waiting for a request in flight.
pub struct Shared {
    pub(crate) status: Mutex<SyncStatus>,
    pub(crate) unlocked: AtomicBool,
}

impl Shared {
    fn new(status: SyncStatus) -> Self {
        Self {
            status: Mutex::new(status),
            unlocked: AtomicBool::new(false),
        }
    }

    pub fn status(&self) -> SyncStatus {
        lock(&self.status).clone()
    }

    pub fn is_unlocked(&self) -> bool {
        self.unlocked.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PullReason {
    Unlock,
    Foreground,
    Poll,
    /// After a rejected push: the local file is dirty by definition.
    Conflict,
}

/// A token returned by the server exactly once, already in secure storage.
#[derive(Debug, Clone, PartialEq)]
pub struct CreatedProfile {
    pub profile: SyncProfile,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JoinOutcome {
    /// The remote vault was validated and is now the live vault (still
    /// locked: unlock it with the master password just verified, then call
    /// `unlocked()` on the handle).
    Joined(SyncProfile),
    /// The slot holds nothing yet; offer `push_local_vault_into_empty_slot`.
    EmptySlot,
}

const TOKEN_MISSING: &str = "The saved sync token is missing. Pair this device again.";

pub struct Coordinator {
    engine: SyncEngine<Arc<dyn Transport>>,
    profiles: SharedProfiles,
    host: Arc<dyn VaultHost>,
    clock: Arc<dyn Clock>,
    config: CoordinatorConfig,
    shared: Arc<Shared>,
    debounce_deadline: Option<Instant>,
    /// `Some` while the 30 s poll runs.
    next_poll: Option<Instant>,
    /// Polls before this instant are skipped (failure backoff).
    next_attempt: Option<Instant>,
    backoff_step: usize,
    /// Set after a `413`: this exact file is not pushed again.
    size_blocked_sha: Option<String>,
    /// Set when a failure that needs the user (rejected token, a refused
    /// vault) stopped the poll; a new unlock or pairing clears it.
    polling_halted: bool,
}

impl Coordinator {
    pub fn new(
        transport: Arc<dyn Transport>,
        profiles: SharedProfiles,
        host: Arc<dyn VaultHost>,
        clock: Arc<dyn Clock>,
        config: CoordinatorConfig,
    ) -> Self {
        let initial = if lock(&profiles).active_profile().is_some() {
            SyncStatus::Idle
        } else {
            SyncStatus::Off
        };
        Self {
            engine: SyncEngine::new(host.vault_path(), transport),
            profiles,
            host,
            clock,
            config,
            shared: Arc::new(Shared::new(initial)),
            debounce_deadline: None,
            next_poll: None,
            next_attempt: None,
            backoff_step: 0,
            size_blocked_sha: None,
            polling_halted: false,
        }
    }

    /// Replaces the engine's copy-name timestamp (local time, or a fixed one
    /// in tests).
    pub fn with_timestamp(
        mut self,
        timestamp: impl Fn() -> String + Send + Sync + 'static,
    ) -> Self {
        self.engine = self.engine.with_timestamp(timestamp);
        self
    }

    pub fn shared(&self) -> Arc<Shared> {
        self.shared.clone()
    }

    pub(crate) fn profiles_handle(&self) -> SharedProfiles {
        self.profiles.clone()
    }

    pub(crate) fn host_handle(&self) -> Arc<dyn VaultHost> {
        self.host.clone()
    }

    pub fn engine(&self) -> &SyncEngine<Arc<dyn Transport>> {
        &self.engine
    }

    pub fn status(&self) -> SyncStatus {
        self.shared.status()
    }

    pub fn is_unlocked(&self) -> bool {
        self.shared.is_unlocked()
    }

    fn active_profile(&self) -> Option<SyncProfile> {
        lock(&self.profiles).active_profile().cloned()
    }

    fn profile(&self, id: Uuid) -> Option<SyncProfile> {
        lock(&self.profiles).profile(id).cloned()
    }

    fn set_status(&self, status: SyncStatus) {
        {
            let mut current = lock(&self.shared.status);
            if *current == status {
                return;
            }
            *current = status.clone();
        }
        self.host.on_status(&status);
    }

    // MARK: - Triggers

    /// The vault just unlocked: pull (a full conditional `GET`) and start
    /// polling.
    pub fn vault_did_unlock(&mut self) {
        self.shared.unlocked.store(true, Ordering::SeqCst);
        if self.active_profile().is_none() {
            self.set_status(SyncStatus::Off);
            return;
        }
        self.set_status(SyncStatus::Idle);
        self.backoff_step = 0;
        self.next_attempt = None;
        self.polling_halted = false;
        self.pull_if_possible(PullReason::Unlock);
        self.ensure_polling();
    }

    /// The vault is about to lock: stop polling, drop a pending debounce and
    /// make one best-effort push of unsaved changes. The push only reads the
    /// file, so it needs no key.
    pub fn vault_will_lock(&mut self) {
        self.shared.unlocked.store(false, Ordering::SeqCst);
        self.stop_polling();
        self.debounce_deadline = None;
        self.flush_for_lock();
    }

    /// Best-effort push when the vault locks; only if there is something new.
    pub fn flush_for_lock(&mut self) {
        if self.active_profile().is_some() && self.local_is_dirty() {
            self.perform_push(true, None);
        }
    }

    /// The vault file was written locally (host edit, password change).
    /// Schedules a push after the debounce; further writes push it back.
    pub fn local_vault_changed(&mut self) {
        if !self.is_unlocked() || self.active_profile().is_none() {
            return;
        }
        self.debounce_deadline = Some(self.clock.now() + self.config.debounce);
    }

    /// The app window regained focus: a cheap `HEAD`-first pull.
    pub fn foreground(&mut self) {
        self.pull_if_possible(PullReason::Foreground);
    }

    /// True when quitting should wait for a last push.
    pub fn wants_flush_before_quit(&self) -> bool {
        self.is_unlocked() && self.active_profile().is_some() && self.local_is_dirty()
    }

    /// The last push before quit, bounded by `timeout` for the request.
    pub fn flush_before_quit(&mut self, timeout: Duration) {
        if self.wants_flush_before_quit() {
            self.perform_push(true, Some(timeout));
        }
    }

    /// Runs whatever is due: the debounced push, then the poll.
    pub fn tick(&mut self) {
        let now = self.clock.now();
        if self.debounce_deadline.is_some_and(|due| due <= now) {
            self.debounce_deadline = None;
            self.perform_push(false, None);
        }
        let now = self.clock.now();
        if self.next_poll.is_some_and(|due| due <= now) {
            self.next_poll = Some(now + self.config.poll_interval);
            self.poll_tick();
        }
    }

    /// The earliest instant `tick()` has work to do, if any.
    pub fn next_wakeup(&self) -> Option<Instant> {
        match (self.debounce_deadline, self.next_poll) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// How long the runner may sleep before `tick()` has work, `None` when
    /// nothing is scheduled.
    pub fn wait_hint(&self) -> Option<Duration> {
        self.next_wakeup()
            .map(|due| due.saturating_duration_since(self.clock.now()))
    }

    fn ensure_polling(&mut self) {
        if self.next_poll.is_none()
            && !self.polling_halted
            && self.is_unlocked()
            && self.active_profile().is_some()
        {
            self.next_poll = Some(self.clock.now() + self.config.poll_interval);
        }
    }

    fn stop_polling(&mut self) {
        self.next_poll = None;
    }

    fn poll_tick(&mut self) {
        if self.active_profile().is_none() || !self.is_unlocked() {
            return;
        }
        if self
            .next_attempt
            .is_some_and(|gate| self.clock.now() < gate)
        {
            return;
        }
        self.pull_if_possible(PullReason::Poll);
    }

    fn schedule_backoff(&mut self) {
        let steps = &self.config.backoff;
        let delay = steps
            .get(self.backoff_step.min(steps.len().saturating_sub(1)))
            .copied()
            .unwrap_or(Duration::from_secs(30));
        self.backoff_step += 1;
        self.next_attempt = Some(self.clock.now() + delay);
    }

    fn local_is_dirty(&self) -> bool {
        match self.active_profile() {
            Some(profile) => self.engine.local_vault_sha256() != profile.last_sha256,
            None => false,
        }
    }

    // MARK: - Pull

    fn token_for(&self, profile: &SyncProfile) -> Result<String, SyncError> {
        match lock(&self.profiles).token(profile.id)? {
            Some(token) if !token.is_empty() => Ok(token),
            _ => Err(SyncError::TokenStore(TOKEN_MISSING.into())),
        }
    }

    fn pull_if_possible(&mut self, reason: PullReason) {
        let Some(profile) = self.active_profile() else {
            return;
        };
        if !self.is_unlocked() {
            return;
        }
        let token = match self.token_for(&profile) {
            Ok(token) => token,
            Err(SyncError::TokenStore(message)) if message == TOKEN_MISSING => {
                self.set_status(SyncStatus::Failed(message));
                return;
            }
            Err(error) => {
                self.set_status(SyncStatus::Failed(error.to_string()));
                return;
            }
        };

        // Polling and foreground checks use HEAD first; 304 almost always.
        if !matches!(reason, PullReason::Conflict | PullReason::Unlock) {
            match self
                .engine
                .head(&profile.server_url, &token, profile.last_version)
            {
                Ok(FetchResult::NotModified(_) | FetchResult::Empty(_)) => {
                    self.set_status(SyncStatus::Idle);
                    return;
                }
                Ok(FetchResult::Current(state)) if state.version == profile.last_version => {
                    self.set_status(SyncStatus::Idle);
                    return;
                }
                Ok(_) => {}
                Err(error) => {
                    self.note(error, true);
                    return;
                }
            }
        }

        let Some(unlocked) = self.host.unlocked() else {
            return;
        };
        self.set_status(SyncStatus::Syncing);
        let dirty = self.local_is_dirty();
        let before = self.read_contents(&unlocked);
        match self.engine.pull(
            &profile.server_url,
            &token,
            profile.last_version,
            dirty,
            &unlocked,
        ) {
            Ok(result) => self.handle_pull(result, &profile, &unlocked, before),
            Err(error) => self.note(error, true),
        }
    }

    fn read_contents(&self, unlocked: &Unlocked) -> Option<Contents> {
        VaultFile::new(self.engine.vault_path())
            .contents(unlocked)
            .ok()
    }

    fn handle_pull(
        &mut self,
        result: PullResult,
        profile: &SyncProfile,
        unlocked: &Unlocked,
        before: Option<Contents>,
    ) {
        match result {
            PullResult::NotModified(state) => {
                if let Some(sha) = state
                    .sha256
                    .filter(|s| profile.last_sha256.as_ref() != Some(s))
                {
                    let _ = lock(&self.profiles).update(profile.id, |p| p.last_sha256 = Some(sha));
                }
                self.set_status(SyncStatus::Idle);
            }
            PullResult::Empty(_) => self.set_status(SyncStatus::Idle),
            PullResult::Adopted {
                state,
                conflict_copy,
            } => {
                if let Err(error) = self.mark_synced(profile.id, &state) {
                    self.set_status(SyncStatus::Failed(error.to_string()));
                }
                let after = self.read_contents(unlocked);
                let report = self.report(state.version, conflict_copy.clone(), before, after);
                self.host.on_remote_adopted(&report);
                match conflict_copy {
                    Some(copy) => {
                        self.set_status(SyncStatus::Conflicted { copy });
                        if let Some((title, message)) = report.banner() {
                            self.host.on_alert(&title, &message);
                        }
                    }
                    None => {
                        if !matches!(self.status(), SyncStatus::Failed(_)) {
                            self.set_status(SyncStatus::Idle);
                        }
                    }
                }
            }
        }
    }

    fn report(
        &self,
        version: u64,
        conflict_copy_name: Option<String>,
        before: Option<Contents>,
        after: Option<Contents>,
    ) -> AdoptReport {
        let dir = self
            .engine
            .vault_path()
            .parent()
            .map(PathBuf::from)
            .unwrap_or_default();
        let (hosts_changed, removed_host_ids) = match (before, after) {
            (Some(before), Some(after)) => {
                let removed = before
                    .hosts
                    .iter()
                    .filter(|old| !after.hosts.iter().any(|new| new.id == old.id))
                    .map(|old| old.id)
                    .collect();
                (before != after, removed)
            }
            _ => (true, Vec::new()),
        };
        AdoptReport {
            version,
            conflict_copy: conflict_copy_name.as_ref().map(|name| dir.join(name)),
            conflict_copy_name,
            hosts_changed,
            removed_host_ids,
        }
    }

    // MARK: - Push

    /// Pushes the live vault if it differs from the last synced bytes.
    /// `allow_locked` is for the lock and quit flushes and the pre-switch
    /// push, which only read the file.
    fn perform_push(&mut self, allow_locked: bool, timeout: Option<Duration>) {
        let Some(profile) = self.active_profile() else {
            return;
        };
        if !(allow_locked || self.is_unlocked()) {
            return;
        }
        let Ok(token) = self.token_for(&profile) else {
            return;
        };
        let Some(current) = self.engine.local_vault_sha256() else {
            return;
        };
        if self.size_blocked_sha.as_deref() == Some(current.as_str()) {
            return;
        }
        if profile.last_sha256.as_deref() == Some(current.as_str()) {
            return;
        }
        self.set_status(SyncStatus::Syncing);
        match self.engine.push(
            &profile.server_url,
            &token,
            profile.last_version,
            profile.last_sha256.as_deref(),
            timeout,
        ) {
            Ok(PushResult::Skipped(state) | PushResult::Stored(state)) => {
                match self.mark_synced(profile.id, &state) {
                    Ok(()) => self.set_status(SyncStatus::Idle),
                    Err(error) => self.set_status(SyncStatus::Failed(error.to_string())),
                }
            }
            Err(error) => self.note(error, false),
        }
    }

    fn mark_synced(&mut self, id: Uuid, state: &SlotState) -> Result<(), SyncError> {
        let now = unix_to_apple(self.clock.unix_seconds());
        let version = state.version;
        let sha = state.sha256.clone();
        lock(&self.profiles).update(id, |p| {
            p.last_version = version;
            if let Some(sha) = sha {
                p.last_sha256 = Some(sha);
            }
            p.last_synced_at = Some(now);
        })?;
        self.backoff_step = 0;
        self.next_attempt = None;
        self.size_blocked_sha = None;
        Ok(())
    }

    // MARK: - Failures

    /// Turns a failure into status and follow-up work, as macOS `note`.
    fn note(&mut self, error: SyncError, during_pull: bool) {
        match &error {
            SyncError::Conflict { .. } if !during_pull => {
                // The local file is dirty by definition here; the pull keeps a
                // copy of it before adopting the server's version. The push is
                // not retried: after adoption the two sides are equal.
                self.pull_if_possible(PullReason::Conflict);
            }
            SyncError::Unauthorized => {
                self.set_status(SyncStatus::TokenRejected);
                self.polling_halted = true;
                self.stop_polling();
            }
            SyncError::Offline(message) => {
                self.set_status(SyncStatus::Offline(message.clone()));
                self.schedule_backoff();
            }
            SyncError::TooLarge => {
                self.size_blocked_sha = self.engine.local_vault_sha256();
                self.set_status(SyncStatus::Failed(error.to_string()));
            }
            SyncError::PreconditionRequired { .. } => {
                self.set_status(SyncStatus::Failed(
                    "The sync client sent an incomplete request. Nothing was changed.".into(),
                ));
            }
            SyncError::NotFound(_) | SyncError::Forbidden(_) | SyncError::Host(_) => {
                self.set_status(SyncStatus::Failed(error.to_string()));
            }
            error if error.is_engine_refusal() => {
                let message = error.to_string();
                self.set_status(SyncStatus::Failed(message.clone()));
                self.host.on_alert("Sync stopped", &message);
                self.polling_halted = true;
                self.stop_polling();
            }
            // Server, Malformed, Io, TokenStore, Profiles, and a Conflict
            // answer to a GET: treat like a 5xx, back off, never block.
            _ => {
                self.set_status(SyncStatus::Failed(error.to_string()));
                self.schedule_backoff();
            }
        }
    }

    // MARK: - Pairing

    fn base_url(raw: &str) -> Result<String, SyncError> {
        normalize_server_url(raw).ok_or_else(|| {
            SyncError::Malformed("Enter a server URL, for example http://127.0.0.1:8484".into())
        })
    }

    fn profile_name(name: &str, url: &str) -> String {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            host_of(url).unwrap_or_else(|| "Sync".into())
        } else {
            trimmed.to_string()
        }
    }

    /// Create a slot: this device's vault becomes version 1 and this profile
    /// becomes the active one. The returned token is shown to the user once;
    /// it is already in secure storage. If another profile was active, its
    /// vault is first copied to its workspace file so switching back is
    /// lossless.
    pub fn create_profile(
        &mut self,
        server: &str,
        name: &str,
        create_secret: Option<&str>,
    ) -> Result<CreatedProfile, SyncError> {
        let url = Self::base_url(server)?;
        let outgoing = self.active_profile();
        let (token, state) = self.engine.create(&url, create_secret)?;
        if let Some(outgoing) = outgoing {
            let _ = self
                .engine
                .copy_local_vault(&self.engine.workspace_path(outgoing.id));
        }
        let mut profile = SyncProfile::new(Self::profile_name(name, &url), url.clone());
        profile.last_version = state.version;
        profile.last_sha256 = state.sha256.clone();
        let stored = {
            let mut profiles = lock(&self.profiles);
            profiles
                .add(profile.clone(), &token)
                .and_then(|()| profiles.set_active(Some(profile.id)))
        };
        if let Err(error) = stored {
            // The token cannot be kept, so the new slot would be unreachable:
            // remove it rather than leave an orphan on the server.
            let _ = self.engine.delete(&url, &token);
            return Err(error);
        }
        self.mark_synced(profile.id, &state)?;
        let profile = self.profile(profile.id).unwrap_or(profile);
        self.set_status(SyncStatus::Idle);
        self.polling_halted = false;
        self.ensure_polling();
        Ok(CreatedProfile { profile, token })
    }

    /// Join an existing slot with its token and the master password of the
    /// vault it holds. Nothing changes until that password has been verified;
    /// a refused download is kept as `passwords.remote-...`. On success the
    /// remote vault is the live vault (still locked): unlock it with
    /// `master_password`, then call `unlocked()` on the handle.
    pub fn join_profile(
        &mut self,
        server: &str,
        token: &str,
        name: &str,
        master_password: &str,
    ) -> Result<JoinOutcome, SyncError> {
        let url = Self::base_url(server)?;
        let token = token.trim();
        let (bytes, state) = match self.engine.prepare_join(&url, token, master_password)? {
            JoinPreparation::EmptySlot => return Ok(JoinOutcome::EmptySlot),
            JoinPreparation::Ready(bytes, state) => (bytes, state),
        };
        let outgoing = self.active_profile();

        // The token goes into secure storage before any file moves; if the
        // switch fails afterwards the profile is removed again.
        let mut profile = SyncProfile::new(Self::profile_name(name, &url), url);
        profile.last_version = state.version;
        profile.last_sha256 = state.sha256.clone();
        lock(&self.profiles).add(profile.clone(), token)?;

        let switched = (|| -> Result<(), SyncError> {
            self.host
                .detach_all_for_vault_switch()
                .map_err(SyncError::Host)?;
            if let Some(outgoing) = &outgoing {
                self.perform_push(true, Some(self.config.switch_push_timeout));
                self.engine
                    .move_local_vault(&self.engine.workspace_path(outgoing.id))?;
            }
            self.engine.adopt_prepared_join(&bytes, &state)?;
            lock(&self.profiles).set_active(Some(profile.id))
        })();
        if let Err(error) = switched {
            let _ = lock(&self.profiles).remove(profile.id);
            return Err(error);
        }
        self.mark_synced(profile.id, &state)?;
        self.stop_polling();
        self.debounce_deadline = None;
        self.shared.unlocked.store(false, Ordering::SeqCst);
        self.set_status(SyncStatus::Idle);
        let profile = self.profile(profile.id).unwrap_or(profile);
        Ok(JoinOutcome::Joined(profile))
    }

    /// Join an empty slot by pushing this device's vault as its first version
    /// (`If-None-Match: *`).
    pub fn push_local_vault_into_empty_slot(
        &mut self,
        server: &str,
        token: &str,
        name: &str,
    ) -> Result<SyncProfile, SyncError> {
        let url = Self::base_url(server)?;
        let token = token.trim();
        if !self.engine.local_vault_exists() {
            return Err(SyncError::NoLocalVault);
        }
        let outgoing = self.active_profile();
        let state = match self.engine.push(&url, token, 0, None, None)? {
            PushResult::Stored(state) | PushResult::Skipped(state) => state,
        };
        if let Some(outgoing) = outgoing {
            let _ = self
                .engine
                .copy_local_vault(&self.engine.workspace_path(outgoing.id));
        }
        let mut profile = SyncProfile::new(Self::profile_name(name, &url), url);
        profile.last_version = state.version;
        profile.last_sha256 = state.sha256.clone();
        {
            let mut profiles = lock(&self.profiles);
            profiles.add(profile.clone(), token)?;
            profiles.set_active(Some(profile.id))?;
        }
        self.mark_synced(profile.id, &state)?;
        self.set_status(SyncStatus::Idle);
        self.polling_halted = false;
        self.ensure_polling();
        Ok(self.profile(profile.id).unwrap_or(profile))
    }

    // MARK: - Token and slot management

    /// Replaces the slot's token. The new token is persisted before this
    /// returns it; if the response is lost the slot is unreachable (the
    /// confirmation dialog says so).
    pub fn rotate_token(&mut self, id: Uuid) -> Result<String, SyncError> {
        let (profile, token) = self.profile_and_token(id)?;
        let new_token = self.engine.rotate(&profile.server_url, &token)?;
        lock(&self.profiles).replace_token(&new_token, id)?;
        if lock(&self.profiles).active_profile_id() == Some(id) {
            self.set_status(SyncStatus::Idle);
        }
        Ok(new_token)
    }

    /// Forgets this device's half of a pairing (profile and token). The slot
    /// and the local vault are untouched.
    pub fn disable(&mut self, id: Uuid) -> Result<(), SyncError> {
        lock(&self.profiles).remove(id)?;
        if self.active_profile().is_none() {
            self.set_status(SyncStatus::Off);
            self.stop_polling();
            self.debounce_deadline = None;
        }
        Ok(())
    }

    /// `DELETE /v1/vault`, then forget the profile. Irreversible.
    pub fn delete_remote_data(&mut self, id: Uuid) -> Result<(), SyncError> {
        let (profile, token) = self.profile_and_token(id)?;
        self.engine.delete(&profile.server_url, &token)?;
        self.disable(id)
    }

    fn profile_and_token(&self, id: Uuid) -> Result<(SyncProfile, String), SyncError> {
        let profile = self.profile(id).ok_or(SyncError::Unauthorized)?;
        let token = lock(&self.profiles)
            .token(id)?
            .filter(|t| !t.is_empty())
            .ok_or(SyncError::Unauthorized)?;
        Ok((profile, token))
    }

    // MARK: - Workspaces

    /// Makes `id` the active profile: the current vault is parked in the
    /// outgoing profile's workspace file (after a bounded push), the target's
    /// workspace file becomes the live vault, and the host is told to lock and
    /// ask for that vault's master password.
    pub fn activate(&mut self, id: Uuid) -> Result<(), SyncError> {
        let current = lock(&self.profiles).active_profile_id();
        if current == Some(id) || self.profile(id).is_none() {
            return Ok(());
        }
        let outgoing = current.and_then(|c| self.profile(c));

        if outgoing.is_some() || self.engine.local_vault_exists() {
            self.host
                .detach_all_for_vault_switch()
                .map_err(SyncError::Host)?;
            if let Some(outgoing) = &outgoing {
                self.perform_push(true, Some(self.config.switch_push_timeout));
                self.engine
                    .move_local_vault(&self.engine.workspace_path(outgoing.id))?;
            } else {
                self.engine.move_local_vault_aside("pre-sync")?;
            }
        }

        self.stop_polling();
        self.debounce_deadline = None;

        let workspace = self.engine.workspace_path(id);
        if workspace.exists() {
            self.engine.adopt_local_vault(&workspace)?;
        }
        lock(&self.profiles).set_active(Some(id))?;
        self.shared.unlocked.store(false, Ordering::SeqCst);
        self.set_status(SyncStatus::Idle);
        self.host.on_workspace_switched();
        Ok(())
    }
}

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;
