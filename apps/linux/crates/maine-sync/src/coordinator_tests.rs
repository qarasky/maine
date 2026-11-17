use super::*;
use crate::engine::sha256_hex;
use crate::profile::{MemoryTokenStore, TokenStore};
use crate::testing::{Call, FakeTransport, ManualClock};
use maine_core::model::Host;
use std::fs;
use std::sync::atomic::AtomicUsize;
use tempfile::TempDir;

const TOKEN: &str = "maine-TOK";
const SERVER: &str = "http://127.0.0.1:8484";

#[derive(Default)]
struct TestHost {
    path: PathBuf,
    unlocked: Mutex<Option<Unlocked>>,
    statuses: Mutex<Vec<SyncStatus>>,
    adopted: Mutex<Vec<AdoptReport>>,
    alerts: Mutex<Vec<(String, String)>>,
    detaches: AtomicUsize,
    detach_error: Mutex<Option<String>>,
    switched: AtomicUsize,
}

impl VaultHost for TestHost {
    fn vault_path(&self) -> PathBuf {
        self.path.clone()
    }
    fn unlocked(&self) -> Option<Unlocked> {
        self.unlocked.lock().unwrap().clone()
    }
    fn on_remote_adopted(&self, report: &AdoptReport) {
        self.adopted.lock().unwrap().push(report.clone());
    }
    fn on_status(&self, status: &SyncStatus) {
        self.statuses.lock().unwrap().push(status.clone());
    }
    fn on_alert(&self, title: &str, message: &str) {
        self.alerts
            .lock()
            .unwrap()
            .push((title.to_string(), message.to_string()));
    }
    fn detach_all_for_vault_switch(&self) -> Result<(), String> {
        self.detaches.fetch_add(1, Ordering::SeqCst);
        match self.detach_error.lock().unwrap().clone() {
            Some(message) => Err(message),
            None => Ok(()),
        }
    }
    fn on_workspace_switched(&self) {
        self.switched.fetch_add(1, Ordering::SeqCst);
    }
}

struct Fixture {
    dir: TempDir,
    file: VaultFile,
    unlocked: Unlocked,
    host: Arc<TestHost>,
    transport: Arc<FakeTransport>,
    clock: Arc<ManualClock>,
    profiles: SharedProfiles,
    coordinator: Coordinator,
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

impl Fixture {
    /// A vault, no profile.
    fn bare() -> Self {
        Self::build(Arc::new(MemoryTokenStore::new()))
    }

    /// A vault whose active profile (version 1) matches the file on disk.
    fn synced() -> Self {
        let mut fx = Self::bare();
        fx.add_profile("Homelab", 1);
        fx
    }

    fn build(tokens: Arc<dyn TokenStore>) -> Self {
        let dir = TempDir::new().unwrap();
        let file = VaultFile::new(dir.path().join("passwords.maine"));
        let unlocked = file.create("coordinator master password").unwrap();
        let host = Arc::new(TestHost {
            path: file.path().to_path_buf(),
            unlocked: Mutex::new(Some(unlocked.clone())),
            ..TestHost::default()
        });
        let transport = Arc::new(FakeTransport::new());
        let clock = Arc::new(ManualClock::new());
        let store_tokens: Arc<dyn TokenStore> = tokens;
        let profiles: SharedProfiles = Arc::new(Mutex::new(
            SyncProfileStore::open(dir.path().join("state"), store_tokens).unwrap(),
        ));
        let coordinator = Coordinator::new(
            transport.clone(),
            profiles.clone(),
            host.clone(),
            clock.clone(),
            CoordinatorConfig::default(),
        )
        .with_timestamp(|| "20260101-000000".to_string());
        Self {
            dir,
            file,
            unlocked,
            host,
            transport,
            clock,
            profiles,
            coordinator,
        }
    }

    fn token_store(&self) -> MutexGuard<'_, SyncProfileStore> {
        lock(&self.profiles)
    }

    fn add_profile(&mut self, name: &str, version: u64) -> SyncProfile {
        let mut profile = SyncProfile::new(name, SERVER);
        profile.last_version = version;
        profile.last_sha256 = self.local_sha();
        let mut store = lock(&self.profiles);
        store.add(profile.clone(), TOKEN).unwrap();
        drop(store);
        // The coordinator was built before the profile existed.
        self.coordinator
            .shared()
            .status
            .lock()
            .map(|mut s| *s = SyncStatus::Idle)
            .unwrap();
        profile
    }

    fn local_sha(&self) -> Option<String> {
        fs::read(self.file.path()).ok().map(|b| sha256_hex(&b))
    }

    fn local_bytes(&self) -> Vec<u8> {
        fs::read(self.file.path()).unwrap()
    }

    fn edit(&self, host: &Host) {
        let host = host.clone();
        self.file
            .update(&self.unlocked, move |c| c.hosts.push(host))
            .unwrap();
    }

    fn active(&self) -> SyncProfile {
        lock(&self.profiles).active_profile().cloned().unwrap()
    }

    fn statuses(&self) -> Vec<SyncStatus> {
        self.host.statuses.lock().unwrap().clone()
    }

    fn adopted(&self) -> Vec<AdoptReport> {
        self.host.adopted.lock().unwrap().clone()
    }

    fn alerts(&self) -> Vec<(String, String)> {
        self.host.alerts.lock().unwrap().clone()
    }

    fn advance(&mut self, seconds: u64) {
        self.clock.advance(secs(seconds));
        self.coordinator.tick();
    }

    /// The bytes of this vault after `edit` was applied to a copy, as a
    /// server would hold a newer version.
    fn newer_remote(&self, add: &Host) -> Vec<u8> {
        let copy_dir = TempDir::new().unwrap();
        let copy = VaultFile::new(copy_dir.path().join("passwords.maine"));
        fs::copy(self.file.path(), copy.path()).unwrap();
        let add = add.clone();
        copy.update(&self.unlocked, move |c| c.hosts.push(add))
            .unwrap();
        fs::read(copy.path()).unwrap()
    }

    fn blob(bytes: &[u8], version: u64) -> FetchResult {
        FetchResult::Blob(
            bytes.to_vec(),
            SlotState::with(version, bytes.len() as u64, sha256_hex(bytes)),
        )
    }

    fn put_ok(&self, version: u64) -> Result<SlotState, SyncError> {
        let bytes = self.local_bytes();
        Ok(SlotState::with(
            version,
            bytes.len() as u64,
            sha256_hex(&bytes),
        ))
    }

    fn names(&self) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(self.dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

fn host(name: &str) -> Host {
    Host::new(name, format!("{name}.example.com"))
}

fn fetch_call(version: u64) -> Call {
    Call::Fetch {
        token: TOKEN.into(),
        if_none_match: Some(version),
    }
}

fn head_call(version: u64) -> Call {
    Call::Head {
        token: TOKEN.into(),
        if_none_match: Some(version),
    }
}

// MARK: - Triggers

#[test]
fn unlock_pulls_once_with_a_conditional_get_and_starts_polling() {
    let mut fx = Fixture::synced();
    fx.transport
        .enqueue_fetch(Ok(FetchResult::NotModified(SlotState::new(1))));

    fx.coordinator.vault_did_unlock();

    assert_eq!(fx.transport.calls(), vec![fetch_call(1)]);
    assert_eq!(fx.coordinator.status(), SyncStatus::Idle);
    assert_eq!(
        fx.statuses(),
        vec![SyncStatus::Syncing, SyncStatus::Idle],
        "no duplicate status callbacks"
    );
    assert!(fx.coordinator.next_wakeup().is_some());

    // Nothing before 30 s, then one cheap HEAD.
    fx.transport.clear_calls();
    fx.advance(29);
    assert!(fx.transport.calls().is_empty());
    fx.transport
        .enqueue_fetch(Ok(FetchResult::NotModified(SlotState::new(1))));
    fx.advance(1);
    assert_eq!(fx.transport.calls(), vec![head_call(1)]);
}

#[test]
fn unlock_without_a_profile_does_nothing() {
    let mut fx = Fixture::bare();

    fx.coordinator.vault_did_unlock();
    fx.edit(&host("a"));
    fx.coordinator.local_vault_changed();
    fx.advance(3600);

    assert_eq!(fx.coordinator.status(), SyncStatus::Off);
    assert!(fx.transport.calls().is_empty());
    assert_eq!(fx.coordinator.next_wakeup(), None);
}

#[test]
fn writes_are_debounced_for_two_seconds_and_coalesced() {
    let mut fx = Fixture::synced();
    fx.coordinator.vault_did_unlock_for_test();
    fx.edit(&host("a"));
    fx.coordinator.local_vault_changed();

    fx.advance(1);
    assert!(fx.transport.calls().is_empty());
    // A second write pushes the deadline back.
    fx.coordinator.local_vault_changed();
    fx.advance(1);
    assert!(fx.transport.calls().is_empty());
    fx.transport.enqueue_put(fx.put_ok(2));
    fx.advance(1);

    let calls = fx.transport.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(
        calls[0],
        Call::Put {
            token: TOKEN.into(),
            body: fx.local_bytes(),
            expected_version: Some(1),
            timeout: None
        }
    );
    let profile = fx.active();
    assert_eq!(profile.last_version, 2);
    assert_eq!(profile.last_sha256, fx.local_sha());
    assert!(profile.last_synced_at.is_some());
    assert_eq!(fx.coordinator.status(), SyncStatus::Idle);
    // The debounce is spent: nothing else fires.
    fx.transport.clear_calls();
    fx.advance(1);
    assert!(fx.transport.calls().iter().all(|c| c.name() != "put"));
}

#[test]
fn a_write_that_changes_nothing_pushes_nothing() {
    let mut fx = Fixture::synced();
    fx.coordinator.vault_did_unlock_for_test();
    fx.coordinator.local_vault_changed();

    fx.advance(2);

    assert!(fx.transport.calls().is_empty());
    assert_eq!(fx.coordinator.status(), SyncStatus::Idle);
}

#[test]
fn a_poll_that_sees_a_newer_version_adopts_it_and_reports_the_change() {
    let mut fx = Fixture::bare();
    let gone = host("gone");
    fx.edit(&gone);
    fx.add_profile("Homelab", 1);
    let remote = fx.newer_remote(&host("fresh"));
    fx.coordinator.vault_did_unlock_for_test();
    let state = SlotState::with(2, remote.len() as u64, sha256_hex(&remote));
    fx.transport.enqueue_fetch(Ok(FetchResult::Current(state)));
    fx.transport.enqueue_fetch(Ok(Fixture::blob(&remote, 2)));

    fx.advance(30);

    assert_eq!(fx.transport.calls(), vec![head_call(1), fetch_call(1)]);
    assert_eq!(fx.local_bytes(), remote);
    let reports = fx.adopted();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].version, 2);
    assert!(reports[0].hosts_changed);
    assert_eq!(reports[0].conflict_copy, None);
    assert!(reports[0].removed_host_ids.is_empty());
    assert_eq!(fx.active().last_version, 2);
    assert_eq!(fx.coordinator.status(), SyncStatus::Idle);
    assert!(fx.alerts().is_empty());
}

#[test]
fn an_adopted_vault_reports_the_hosts_it_removed() {
    let mut fx = Fixture::bare();
    let original = fx.local_bytes();
    let gone = host("gone");
    fx.edit(&gone);
    fx.add_profile("Homelab", 1);
    // The server's copy descends from the empty vault: `gone` is not in it.
    let mut remote = original;
    {
        let copy_dir = TempDir::new().unwrap();
        let copy = VaultFile::new(copy_dir.path().join("passwords.maine"));
        fs::write(copy.path(), &remote).unwrap();
        copy.update(&fx.unlocked, |c| c.hosts.push(host("kept")))
            .unwrap();
        remote = fs::read(copy.path()).unwrap();
    }
    fx.coordinator.vault_did_unlock_for_test();
    fx.transport.enqueue_fetch(Ok(Fixture::blob(&remote, 2)));

    fx.coordinator.foreground_for_test_get();

    let report = fx.adopted().remove(0);
    assert!(report.hosts_changed);
    assert_eq!(report.removed_host_ids, vec![gone.id]);
}

#[test]
fn head_answers_that_need_no_get_leave_the_status_alone() {
    let mut fx = Fixture::synced();
    fx.coordinator.vault_did_unlock_for_test();
    fx.transport
        .enqueue_fetch(Ok(FetchResult::NotModified(SlotState::new(1))));
    fx.transport
        .enqueue_fetch(Ok(FetchResult::Empty(SlotState::new(0))));
    fx.transport
        .enqueue_fetch(Ok(FetchResult::Current(SlotState::new(1))));

    fx.advance(30);
    fx.advance(30);
    fx.advance(30);

    assert_eq!(
        fx.transport.call_names(),
        vec!["head", "head", "head"],
        "no GET was needed"
    );
    assert!(!fx.statuses().contains(&SyncStatus::Syncing));
    assert_eq!(fx.coordinator.status(), SyncStatus::Idle);
}

#[test]
fn offline_polls_back_off_30_120_300_seconds() {
    let mut fx = Fixture::synced();
    let offline = || Err(SyncError::Offline("No network connection.".into()));
    fx.transport.enqueue_fetch(offline()); // unlock pull at t=0
    fx.coordinator.vault_did_unlock();
    assert_eq!(
        fx.coordinator.status(),
        SyncStatus::Offline("No network connection.".into())
    );
    assert_eq!(fx.transport.calls().len(), 1);

    // t=30: allowed (gate 30), fails, next gate t=150.
    fx.transport.enqueue_fetch(offline());
    fx.advance(30);
    assert_eq!(fx.transport.calls().len(), 2);
    for _ in 0..3 {
        fx.advance(30); // t=60, 90, 120: skipped
    }
    assert_eq!(fx.transport.calls().len(), 2);

    // t=150: allowed, fails, next gate t=450.
    fx.transport.enqueue_fetch(offline());
    fx.advance(30);
    assert_eq!(fx.transport.calls().len(), 3);
    for _ in 0..9 {
        fx.advance(30); // t=180..420: skipped
    }
    assert_eq!(fx.transport.calls().len(), 3);

    // t=450: allowed, fails, the cap stays 300 s (gate t=750).
    fx.transport.enqueue_fetch(offline());
    fx.advance(30);
    assert_eq!(fx.transport.calls().len(), 4);
    for _ in 0..9 {
        fx.advance(30); // t=480..720: skipped
    }
    assert_eq!(fx.transport.calls().len(), 4);

    // t=750 recovers; the next poll (t=780) runs at once.
    fx.transport
        .enqueue_fetch(Ok(FetchResult::NotModified(SlotState::new(1))));
    fx.advance(30);
    assert_eq!(fx.coordinator.status(), SyncStatus::Idle);
    fx.transport
        .enqueue_fetch(Ok(FetchResult::NotModified(SlotState::new(1))));
    fx.advance(30);
    assert_eq!(fx.transport.calls().len(), 6);
}

#[test]
fn locking_makes_one_best_effort_push_and_stops_everything() {
    let mut fx = Fixture::synced();
    fx.coordinator.vault_did_unlock_for_test();
    fx.edit(&host("a"));
    fx.coordinator.local_vault_changed();
    fx.transport.enqueue_put(fx.put_ok(2));

    fx.coordinator.vault_will_lock();

    assert_eq!(fx.transport.call_names(), vec!["put"]);
    assert!(!fx.coordinator.is_unlocked());
    assert_eq!(fx.coordinator.next_wakeup(), None, "no poll, no debounce");
    fx.transport.clear_calls();
    fx.advance(600);
    assert!(fx.transport.calls().is_empty());
}

#[test]
fn locking_a_clean_vault_pushes_nothing() {
    let mut fx = Fixture::synced();
    fx.coordinator.vault_did_unlock_for_test();

    fx.coordinator.vault_will_lock();

    assert!(fx.transport.calls().is_empty());
}

#[test]
fn quit_flushes_dirty_changes_with_the_given_timeout() {
    let mut fx = Fixture::synced();
    fx.coordinator.vault_did_unlock_for_test();
    assert!(!fx.coordinator.wants_flush_before_quit());
    fx.coordinator.flush_before_quit(secs(2));
    assert!(fx.transport.calls().is_empty());

    fx.edit(&host("a"));
    assert!(fx.coordinator.wants_flush_before_quit());
    fx.transport.enqueue_put(fx.put_ok(2));
    fx.coordinator.flush_before_quit(secs(2));

    match &fx.transport.calls()[0] {
        Call::Put { timeout, .. } => assert_eq!(*timeout, Some(secs(2))),
        other => panic!("expected a put, got {other:?}"),
    }
    assert!(!fx.coordinator.wants_flush_before_quit());
}

#[test]
fn a_locked_vault_neither_polls_nor_pushes_nor_pulls() {
    let mut fx = Fixture::synced();
    fx.edit(&host("a"));

    fx.coordinator.local_vault_changed();
    fx.coordinator.foreground();
    fx.advance(3600);

    assert!(fx.transport.calls().is_empty());
    assert!(!fx.coordinator.wants_flush_before_quit());
}

#[test]
fn foreground_checks_with_a_head_first() {
    let mut fx = Fixture::synced();
    fx.coordinator.vault_did_unlock_for_test();
    fx.transport
        .enqueue_fetch(Ok(FetchResult::NotModified(SlotState::new(1))));

    fx.coordinator.foreground();

    assert_eq!(fx.transport.calls(), vec![head_call(1)]);
}

// MARK: - Conflicts and failures

#[test]
fn a_rejected_push_pulls_keeps_a_conflict_copy_and_does_not_retry() {
    let mut fx = Fixture::synced();
    fx.coordinator.vault_did_unlock_for_test();
    fx.edit(&host("mine"));
    let mine = fx.local_bytes();
    let remote = fx.newer_remote(&host("theirs"));
    fx.coordinator.local_vault_changed();
    fx.transport
        .enqueue_put(Err(SyncError::Conflict { current_version: 9 }));
    fx.transport.enqueue_fetch(Ok(Fixture::blob(&remote, 9)));

    fx.advance(2);

    assert_eq!(fx.transport.call_names(), vec!["put", "fetch"]);
    assert_eq!(fx.local_bytes(), remote);
    let name = "passwords.conflict-20260101-000000.maine";
    assert_eq!(fs::read(fx.dir.path().join(name)).unwrap(), mine);
    assert_eq!(
        fx.coordinator.status(),
        SyncStatus::Conflicted { copy: name.into() }
    );
    assert_eq!(
        SyncStatus::Conflicted { copy: name.into() }.label(),
        format!("Conflict saved as {name}")
    );
    let report = fx.adopted().remove(0);
    assert_eq!(report.conflict_copy_name.as_deref(), Some(name));
    assert_eq!(report.conflict_copy, Some(fx.dir.path().join(name)));
    assert!(report.hosts_changed);
    assert_eq!(
        fx.alerts(),
        vec![(
            "Another device changed the vault".to_string(),
            format!("Your version was saved as {name}. This device now follows the server's copy.")
        )]
    );
    assert_eq!(fx.active().last_version, 9);
    // No automatic retry of the push.
    fx.transport.clear_calls();
    fx.advance(2);
    assert!(fx.transport.calls().iter().all(|c| c.name() != "put"));
}

#[test]
fn a_dirty_vault_at_unlock_is_kept_as_a_conflict_copy() {
    let mut fx = Fixture::synced();
    fx.edit(&host("mine"));
    let mine = fx.local_bytes();
    let remote = fx.newer_remote(&host("theirs"));
    fx.transport.enqueue_fetch(Ok(Fixture::blob(&remote, 2)));

    fx.coordinator.vault_did_unlock();

    assert_eq!(fx.transport.call_names(), vec!["fetch"]);
    let name = "passwords.conflict-20260101-000000.maine";
    assert_eq!(fs::read(fx.dir.path().join(name)).unwrap(), mine);
    assert_eq!(fx.alerts().len(), 1);
}

#[test]
fn a_rejected_token_stops_polling_until_something_tries_again() {
    let mut fx = Fixture::synced();
    fx.coordinator.vault_did_unlock_for_test();
    fx.transport.enqueue_fetch(Err(SyncError::Unauthorized));

    fx.advance(30);

    assert_eq!(fx.coordinator.status(), SyncStatus::TokenRejected);
    assert_eq!(SyncStatus::TokenRejected.label(), "Token rejected");
    assert_eq!(fx.coordinator.next_wakeup(), None);
    fx.transport.clear_calls();
    fx.advance(3600);
    assert!(fx.transport.calls().is_empty());

    // A foreground check tries again and sees the same answer, quietly.
    fx.transport.enqueue_fetch(Err(SyncError::Unauthorized));
    let before = fx.statuses().len();
    fx.coordinator.foreground();
    assert_eq!(fx.transport.call_names(), vec!["head"]);
    assert_eq!(fx.statuses().len(), before);
}

#[test]
fn a_vault_too_large_for_the_server_is_not_pushed_again_until_it_changes() {
    let mut fx = Fixture::synced();
    fx.coordinator.vault_did_unlock_for_test();
    fx.edit(&host("big"));
    fx.coordinator.local_vault_changed();
    fx.transport.enqueue_put(Err(SyncError::TooLarge));

    fx.advance(2);

    assert_eq!(
        fx.coordinator.status(),
        SyncStatus::Failed("The vault is larger than this server accepts.".into())
    );
    fx.transport.clear_calls();
    fx.coordinator.local_vault_changed();
    fx.advance(2);
    assert!(fx.transport.calls().is_empty(), "same bytes, no retry");

    fx.edit(&host("smaller"));
    fx.transport.enqueue_put(fx.put_ok(2));
    fx.coordinator.local_vault_changed();
    fx.advance(2);
    assert_eq!(fx.transport.call_names(), vec!["put"]);
    assert_eq!(fx.coordinator.status(), SyncStatus::Idle);
}

#[test]
fn a_missing_precondition_is_reported_as_a_client_bug() {
    let mut fx = Fixture::synced();
    fx.coordinator.vault_did_unlock_for_test();
    fx.edit(&host("a"));
    fx.coordinator.local_vault_changed();
    fx.transport
        .enqueue_put(Err(SyncError::PreconditionRequired { current_version: 3 }));

    fx.advance(2);

    assert_eq!(
        fx.coordinator.status(),
        SyncStatus::Failed(
            "The sync client sent an incomplete request. Nothing was changed.".into()
        )
    );
}

#[test]
fn a_foreign_vault_stops_sync_with_an_alert_and_keeps_the_local_file() {
    let mut fx = Fixture::synced();
    let before = fx.local_bytes();
    let other_dir = TempDir::new().unwrap();
    let other = VaultFile::new(other_dir.path().join("passwords.maine"));
    other.create("another master password").unwrap();
    let other_bytes = fs::read(other.path()).unwrap();
    fx.transport
        .enqueue_fetch(Ok(Fixture::blob(&other_bytes, 2)));

    fx.coordinator.vault_did_unlock();

    assert_eq!(fx.local_bytes(), before);
    match fx.coordinator.status() {
        SyncStatus::Failed(message) => {
            assert!(message.contains("different master password"), "{message}");
            assert!(message.contains("passwords.remote-20260101-000000.maine"));
        }
        other => panic!("expected a failure, got {other:?}"),
    }
    assert_eq!(fx.alerts().len(), 1);
    assert_eq!(fx.alerts()[0].0, "Sync stopped");
    assert_eq!(fx.coordinator.next_wakeup(), None, "polling stopped");
    assert!(fx
        .names()
        .contains(&"passwords.remote-20260101-000000.maine".to_string()));
}

#[test]
fn a_missing_token_is_a_status_not_a_request() {
    let fx = Fixture::bare();
    let mut fx = fx;
    let mut profile = SyncProfile::new("Homelab", SERVER);
    profile.last_sha256 = fx.local_sha();
    fx.token_store().add(profile, "").unwrap();

    fx.coordinator.vault_did_unlock();

    assert_eq!(
        fx.coordinator.status(),
        SyncStatus::Failed("The saved sync token is missing. Pair this device again.".into())
    );
    assert!(fx.transport.calls().is_empty());
}

#[test]
fn server_and_malformed_failures_back_off_but_not_found_does_not() {
    for (error, backs_off) in [
        (
            SyncError::Server {
                status: 500,
                message: "database is locked".into(),
            },
            true,
        ),
        (SyncError::Malformed("garbled".into()), true),
        (SyncError::NotFound("nothing".into()), false),
        (SyncError::Forbidden("no".into()), false),
    ] {
        let mut fx = Fixture::synced();
        fx.coordinator.vault_did_unlock_for_test();
        let message = error.to_string();
        let message_for_failure = message.clone();
        fx.transport.enqueue_fetch(Err(error));
        fx.advance(30);
        assert_eq!(fx.coordinator.status(), SyncStatus::Failed(message));

        for _ in 0..2 {
            fx.transport
                .enqueue_fetch(Err(SyncError::Malformed("again".into())));
        }
        fx.advance(30);
        fx.advance(30);
        // A back-off gate of 30 s, then 2 min, skips the third poll.
        let expected = if backs_off { 2 } else { 3 };
        assert_eq!(
            fx.transport.calls().len(),
            expected,
            "{message_for_failure}"
        );
    }
}

#[test]
fn a_status_is_only_reported_when_it_changes() {
    let mut fx = Fixture::synced();
    fx.coordinator.vault_did_unlock_for_test();
    for _ in 0..3 {
        fx.transport.enqueue_fetch(Err(SyncError::Offline(
            "The sync server could not be reached.".into(),
        )));
    }

    fx.advance(30);
    fx.advance(120);
    fx.advance(300);

    let offline = fx
        .statuses()
        .into_iter()
        .filter(|s| matches!(s, SyncStatus::Offline(_)))
        .count();
    assert_eq!(offline, 1);
}

#[test]
fn a_304_with_a_checksum_records_it_and_leaves_the_rest() {
    let mut fx = Fixture::synced();
    fx.transport
        .enqueue_fetch(Ok(FetchResult::NotModified(SlotState {
            version: 1,
            sha256: Some("cafe".into()),
            ..SlotState::default()
        })));

    fx.coordinator.vault_did_unlock();

    assert_eq!(fx.active().last_sha256.as_deref(), Some("cafe"));
    assert_eq!(fx.active().last_version, 1);
}

#[test]
fn status_labels_match_macos() {
    assert_eq!(SyncStatus::Off.label(), "Sync off");
    assert_eq!(SyncStatus::Idle.label(), "Synced");
    assert_eq!(SyncStatus::Syncing.label(), "Syncing\u{2026}");
    assert_eq!(
        SyncStatus::Offline("No network connection.".into()).label(),
        "No network connection."
    );
    assert_eq!(SyncStatus::Failed("boom".into()).label(), "boom");
}

// MARK: - Pairing

#[test]
fn create_profile_stores_the_token_activates_and_starts_polling() {
    let mut fx = Fixture::bare();
    fx.coordinator.vault_did_unlock_for_test();
    let bytes = fx.local_bytes();
    let state = SlotState::with(1, bytes.len() as u64, sha256_hex(&bytes));
    fx.transport
        .enqueue_create(Ok(("maine-NEWTOKEN".into(), state)));

    let created = fx
        .coordinator
        .create_profile("127.0.0.1:8484", "", Some("s3cret"))
        .unwrap();

    assert_eq!(created.token, "maine-NEWTOKEN");
    assert_eq!(created.profile.server_url, "http://127.0.0.1:8484");
    assert_eq!(created.profile.name, "127.0.0.1");
    assert_eq!(created.profile.last_version, 1);
    assert!(created.profile.last_synced_at.is_some());
    let store = fx.token_store();
    assert_eq!(store.active_profile_id(), Some(created.profile.id));
    assert_eq!(
        store.token(created.profile.id).unwrap().as_deref(),
        Some("maine-NEWTOKEN")
    );
    drop(store);
    assert_eq!(
        fx.transport.calls(),
        vec![Call::Create {
            create_secret: Some("s3cret".into()),
            body: bytes
        }]
    );
    assert_eq!(fx.coordinator.status(), SyncStatus::Idle);
    assert!(
        fx.coordinator.next_wakeup().is_some(),
        "polling starts in the same session"
    );
}

#[test]
fn create_profile_parks_the_outgoing_profiles_vault() {
    let mut fx = Fixture::synced();
    let outgoing = fx.active();
    let bytes = fx.local_bytes();
    fx.transport.enqueue_create(Ok((
        "maine-NEW".into(),
        SlotState::with(1, bytes.len() as u64, sha256_hex(&bytes)),
    )));

    let created = fx
        .coordinator
        .create_profile(SERVER, "Second", None)
        .unwrap();

    let workspace = fx.coordinator.engine().workspace_path(outgoing.id);
    assert_eq!(fs::read(workspace).unwrap(), bytes);
    assert!(fx.file.exists());
    // The old profile stays; the new one is active.
    assert_eq!(fx.token_store().profiles().len(), 2);
    assert_eq!(
        fx.token_store().active_profile_id(),
        Some(created.profile.id)
    );
    assert_eq!(created.profile.name, "Second");
}

#[test]
fn create_profile_failures_leave_no_profile() {
    let mut fx = Fixture::bare();
    fx.transport.enqueue_create(Err(SyncError::Forbidden(
        "a create secret is required".into(),
    )));

    let error = fx
        .coordinator
        .create_profile(SERVER, "x", None)
        .unwrap_err();

    assert_eq!(
        error,
        SyncError::Forbidden("a create secret is required".into())
    );
    assert!(!fx.token_store().has_profiles());
    assert_eq!(
        fx.coordinator
            .create_profile("ftp://nope", "x", None)
            .unwrap_err(),
        SyncError::Malformed("Enter a server URL, for example http://127.0.0.1:8484".into())
    );
    assert_eq!(fx.transport.calls().len(), 1);
}

struct FailingTokens;

impl TokenStore for FailingTokens {
    fn get(&self, _: Uuid) -> Result<Option<String>, SyncError> {
        Ok(None)
    }
    fn set(&self, _: Uuid, _: &str) -> Result<(), SyncError> {
        Err(SyncError::TokenStore("the keyring is locked".into()))
    }
    fn delete(&self, _: Uuid) -> Result<(), SyncError> {
        Ok(())
    }
}

#[test]
fn a_token_that_cannot_be_stored_removes_the_new_slot_again() {
    let mut fx = Fixture::build(Arc::new(FailingTokens));
    fx.transport
        .enqueue_create(Ok(("maine-LOST".into(), SlotState::new(1))));
    fx.transport.enqueue_delete(Ok(()));

    let error = fx
        .coordinator
        .create_profile(SERVER, "x", None)
        .unwrap_err();

    assert_eq!(error, SyncError::TokenStore("the keyring is locked".into()));
    assert_eq!(fx.transport.call_names(), vec!["create", "delete"]);
    assert!(!fx.token_store().has_profiles());
}

fn remote_vault(password: &str) -> (TempDir, Vec<u8>) {
    let dir = TempDir::new().unwrap();
    let file = VaultFile::new(dir.path().join("passwords.maine"));
    file.create(password).unwrap();
    let bytes = fs::read(file.path()).unwrap();
    (dir, bytes)
}

#[test]
fn join_adopts_the_remote_vault_and_parks_the_outgoing_one() {
    let mut fx = Fixture::synced();
    let outgoing = fx.active();
    fx.edit(&host("unpushed"));
    let (_keep, remote) = remote_vault("remote master password");
    fx.transport.enqueue_fetch(Ok(Fixture::blob(&remote, 4)));
    fx.transport.enqueue_put(fx.put_ok(2));
    let pushed = fx.local_bytes();

    let outcome = fx
        .coordinator
        .join_profile(
            "sync.example.com:9",
            "  maine-JOIN  ",
            "",
            "remote master password",
        )
        .unwrap();

    let joined = match outcome {
        JoinOutcome::Joined(profile) => profile,
        other => panic!("expected a join, got {other:?}"),
    };
    assert_eq!(joined.server_url, "http://sync.example.com:9");
    assert_eq!(joined.name, "sync.example.com");
    assert_eq!(joined.last_version, 4);
    assert_eq!(fx.local_bytes(), remote);
    // The outgoing vault was pushed (bounded) and parked, byte for byte.
    assert_eq!(
        fx.transport.calls()[1],
        Call::Put {
            token: TOKEN.into(),
            body: pushed.clone(),
            expected_version: Some(1),
            timeout: Some(secs(3))
        }
    );
    assert_eq!(
        fs::read(fx.coordinator.engine().workspace_path(outgoing.id)).unwrap(),
        pushed
    );
    let store = fx.token_store();
    assert_eq!(store.active_profile_id(), Some(joined.id));
    assert_eq!(
        store.token(joined.id).unwrap().as_deref(),
        Some("maine-JOIN")
    );
    assert_eq!(store.profile(outgoing.id).unwrap().last_version, 2);
    drop(store);
    assert_eq!(fx.host.detaches.load(Ordering::SeqCst), 1);
    assert_eq!(fx.coordinator.status(), SyncStatus::Idle);
    assert!(
        !fx.coordinator.is_unlocked(),
        "the app unlocks the adopted vault"
    );
}

#[test]
fn join_without_a_profile_keeps_the_local_vault_as_a_pre_sync_copy() {
    let mut fx = Fixture::bare();
    let local = fx.local_bytes();
    let (_keep, remote) = remote_vault("remote master password");
    fx.transport.enqueue_fetch(Ok(Fixture::blob(&remote, 1)));

    fx.coordinator
        .join_profile(SERVER, TOKEN, "Named", "remote master password")
        .unwrap();

    assert_eq!(fx.local_bytes(), remote);
    let copy = fx
        .names()
        .into_iter()
        .find(|n| n.starts_with("passwords.pre-sync-"))
        .expect("a pre-sync copy");
    assert_eq!(fs::read(fx.dir.path().join(copy)).unwrap(), local);
    assert_eq!(fx.token_store().active_profile().unwrap().name, "Named");
}

#[test]
fn join_with_the_wrong_password_changes_nothing() {
    let mut fx = Fixture::synced();
    let local = fx.local_bytes();
    let (_keep, remote) = remote_vault("remote master password");
    fx.transport.enqueue_fetch(Ok(Fixture::blob(&remote, 4)));

    let error = fx
        .coordinator
        .join_profile(SERVER, TOKEN, "x", "definitely wrong")
        .unwrap_err();

    assert!(matches!(error, SyncError::WrongMasterPassword { .. }));
    assert_eq!(fx.local_bytes(), local);
    assert_eq!(fx.token_store().profiles().len(), 1);
    assert_eq!(fx.host.detaches.load(Ordering::SeqCst), 0);
    assert!(fx
        .names()
        .contains(&"passwords.remote-20260101-000000.maine".to_string()));
}

#[test]
fn join_into_an_empty_slot_offers_the_push_and_changes_nothing() {
    let mut fx = Fixture::bare();
    fx.transport
        .enqueue_fetch(Ok(FetchResult::Empty(SlotState::new(0))));

    let outcome = fx
        .coordinator
        .join_profile(SERVER, TOKEN, "x", "whatever")
        .unwrap();

    assert_eq!(outcome, JoinOutcome::EmptySlot);
    assert!(!fx.token_store().has_profiles());
}

#[test]
fn a_refused_vault_switch_rolls_a_join_back() {
    let mut fx = Fixture::synced();
    let local = fx.local_bytes();
    *fx.host.detach_error.lock().unwrap() = Some("a session could not be saved".into());
    let (_keep, remote) = remote_vault("remote master password");
    fx.transport.enqueue_fetch(Ok(Fixture::blob(&remote, 4)));

    let error = fx
        .coordinator
        .join_profile(SERVER, "maine-JOIN", "x", "remote master password")
        .unwrap_err();

    assert_eq!(
        error,
        SyncError::Host("a session could not be saved".into())
    );
    assert_eq!(fx.local_bytes(), local);
    assert_eq!(fx.token_store().profiles().len(), 1);
    let store = fx.token_store();
    let stray = store
        .profiles()
        .iter()
        .any(|p| store.token(p.id).unwrap().as_deref() == Some("maine-JOIN"));
    assert!(!stray, "the rolled-back token is gone");
}

#[test]
fn pushing_into_an_empty_slot_uses_if_none_match() {
    let mut fx = Fixture::bare();
    fx.transport.enqueue_put(fx.put_ok(1));

    let profile = fx
        .coordinator
        .push_local_vault_into_empty_slot(SERVER, " maine-EMPTY ", "Mine")
        .unwrap();

    match &fx.transport.calls()[0] {
        Call::Put {
            token,
            expected_version,
            ..
        } => {
            assert_eq!(token, "maine-EMPTY");
            assert_eq!(*expected_version, None);
        }
        other => panic!("expected a put, got {other:?}"),
    }
    assert_eq!(profile.last_version, 1);
    assert_eq!(fx.token_store().active_profile_id(), Some(profile.id));
    assert_eq!(fx.coordinator.status(), SyncStatus::Idle);
}

#[test]
fn pushing_into_an_empty_slot_needs_a_local_vault() {
    let mut fx = Fixture::bare();
    fs::remove_file(fx.file.path()).unwrap();

    let error = fx
        .coordinator
        .push_local_vault_into_empty_slot(SERVER, TOKEN, "x")
        .unwrap_err();

    assert_eq!(error, SyncError::NoLocalVault);
    assert!(fx.transport.calls().is_empty());
}

#[test]
fn rotate_persists_the_new_token_and_failure_keeps_the_old_one() {
    let mut fx = Fixture::synced();
    let id = fx.active().id;
    fx.transport.enqueue_rotate(Ok("maine-ROTATED".into()));

    let token = fx.coordinator.rotate_token(id).unwrap();

    assert_eq!(token, "maine-ROTATED");
    assert_eq!(
        fx.transport.calls(),
        vec![Call::Rotate {
            token: TOKEN.into()
        }]
    );
    assert_eq!(
        fx.token_store().token(id).unwrap().as_deref(),
        Some("maine-ROTATED")
    );

    fx.transport.enqueue_rotate(Err(SyncError::Unauthorized));
    assert_eq!(
        fx.coordinator.rotate_token(id).unwrap_err(),
        SyncError::Unauthorized
    );
    assert_eq!(
        fx.token_store().token(id).unwrap().as_deref(),
        Some("maine-ROTATED")
    );

    assert_eq!(
        fx.coordinator.rotate_token(Uuid::new_v4()).unwrap_err(),
        SyncError::Unauthorized
    );
}

#[test]
fn disable_forgets_the_pairing_and_leaves_the_vault_alone() {
    let mut fx = Fixture::synced();
    fx.coordinator.vault_did_unlock_for_test();
    let id = fx.active().id;
    let bytes = fx.local_bytes();

    fx.coordinator.disable(id).unwrap();

    assert!(!fx.token_store().has_profiles());
    assert_eq!(fx.token_store().token(id).unwrap(), None);
    assert_eq!(fx.coordinator.status(), SyncStatus::Off);
    assert_eq!(fx.coordinator.next_wakeup(), None);
    assert_eq!(fx.local_bytes(), bytes);
    assert!(fx.transport.calls().is_empty());
}

#[test]
fn delete_remote_data_wipes_the_slot_then_forgets_the_profile() {
    let mut fx = Fixture::synced();
    let id = fx.active().id;
    fx.transport.enqueue_delete(Err(SyncError::Unauthorized));
    assert_eq!(
        fx.coordinator.delete_remote_data(id).unwrap_err(),
        SyncError::Unauthorized
    );
    assert!(
        fx.token_store().has_profiles(),
        "a failed delete keeps the profile"
    );

    fx.transport.enqueue_delete(Ok(()));
    fx.coordinator.delete_remote_data(id).unwrap();

    assert_eq!(fx.transport.call_names(), vec!["delete", "delete"]);
    assert!(!fx.token_store().has_profiles());
    assert!(fx.file.exists());
}

// MARK: - Workspaces

#[test]
fn activate_swaps_workspace_files_and_asks_the_host_to_lock() {
    let mut fx = Fixture::synced();
    let first = fx.active();
    fx.coordinator.vault_did_unlock_for_test();
    // The second profile has a parked vault of its own.
    let (_keep, other) = remote_vault("other master password");
    let mut second = SyncProfile::new("Second", "http://other.example.com");
    second.last_version = 3;
    fx.token_store()
        .add(second.clone(), "maine-SECOND")
        .unwrap();
    fs::write(fx.coordinator.engine().workspace_path(second.id), &other).unwrap();
    // The first vault has an unpushed edit.
    fx.edit(&host("unpushed"));
    let first_bytes = fx.local_bytes();
    fx.transport.enqueue_put(fx.put_ok(2));

    fx.coordinator.activate(second.id).unwrap();

    assert_eq!(fx.local_bytes(), other);
    assert_eq!(
        fs::read(fx.coordinator.engine().workspace_path(first.id)).unwrap(),
        first_bytes
    );
    assert!(!fx.coordinator.engine().workspace_path(second.id).exists());
    assert_eq!(fx.token_store().active_profile_id(), Some(second.id));
    assert_eq!(fx.token_store().profile(first.id).unwrap().last_version, 2);
    match &fx.transport.calls()[0] {
        Call::Put { timeout, .. } => assert_eq!(*timeout, Some(secs(3))),
        other => panic!("expected a bounded push, got {other:?}"),
    }
    assert_eq!(fx.host.detaches.load(Ordering::SeqCst), 1);
    assert_eq!(fx.host.switched.load(Ordering::SeqCst), 1);
    assert!(!fx.coordinator.is_unlocked());
    assert_eq!(fx.coordinator.next_wakeup(), None);
    assert_eq!(fx.coordinator.status(), SyncStatus::Idle);

    // Activating the active profile is a no-op.
    fx.coordinator.activate(second.id).unwrap();
    assert_eq!(fx.host.switched.load(Ordering::SeqCst), 1);
}

#[test]
fn activate_stops_before_touching_files_when_the_host_refuses() {
    let mut fx = Fixture::synced();
    let second = SyncProfile::new("Second", "http://other.example.com");
    fx.token_store().add(second.clone(), "t").unwrap();
    let bytes = fx.local_bytes();
    let active = fx.active().id;
    *fx.host.detach_error.lock().unwrap() = Some("busy".into());

    let error = fx.coordinator.activate(second.id).unwrap_err();

    assert_eq!(error, SyncError::Host("busy".into()));
    assert_eq!(fx.local_bytes(), bytes);
    assert_eq!(fx.token_store().active_profile_id(), Some(active));
    assert_eq!(fx.host.switched.load(Ordering::SeqCst), 0);
}

#[test]
fn activate_without_an_outgoing_profile_keeps_the_live_vault_as_a_pre_sync_copy() {
    let mut fx = Fixture::bare();
    let local = fx.local_bytes();
    let (_keep, other) = remote_vault("other master password");
    let target = SyncProfile::new("Parked", SERVER);
    fx.token_store().add(target.clone(), "t").unwrap();
    // `add` made it active; clear that to model "no profile active".
    fx.token_store().set_active(None).unwrap();
    fs::write(fx.coordinator.engine().workspace_path(target.id), &other).unwrap();

    fx.coordinator.activate(target.id).unwrap();

    assert_eq!(fx.local_bytes(), other);
    let copy = fx
        .names()
        .into_iter()
        .find(|n| n.starts_with("passwords.pre-sync-"))
        .expect("a pre-sync copy");
    assert_eq!(fs::read(fx.dir.path().join(copy)).unwrap(), local);
    assert_eq!(fx.token_store().active_profile_id(), Some(target.id));
}

// Test-only conveniences on the coordinator.
impl Coordinator {
    /// Marks the vault unlocked and starts polling without the unlock pull.
    fn vault_did_unlock_for_test(&mut self) {
        self.shared.unlocked.store(true, Ordering::SeqCst);
        self.ensure_polling();
    }

    /// A foreground-style pull that skips the HEAD (one queued blob).
    fn foreground_for_test_get(&mut self) {
        self.pull_if_possible(PullReason::Conflict);
    }
}
