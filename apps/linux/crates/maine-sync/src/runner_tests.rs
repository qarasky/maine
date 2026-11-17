use super::*;
use crate::coordinator::AdoptReport;
use crate::engine::sha256_hex;
use crate::profile::MemoryTokenStore;
use crate::testing::FakeTransport;
use crate::time::SystemClock;
use crate::transport::{FetchResult, SlotState, Transport, VersionInfo};
use maine_core::model::Host;
use maine_core::vault::{Unlocked, VaultFile};
use std::path::PathBuf;
use std::time::Instant;
use tempfile::TempDir;

/// Delegates to a fake but sleeps before answering a `put`, to model a slow
/// server.
struct SlowTransport {
    inner: Arc<FakeTransport>,
    put_delay: Duration,
}

impl Transport for SlowTransport {
    fn create_vault(
        &self,
        server: &str,
        secret: Option<&str>,
        body: &[u8],
    ) -> Result<(String, SlotState), SyncError> {
        self.inner.create_vault(server, secret, body)
    }
    fn fetch_vault(&self, s: &str, t: &str, n: Option<u64>) -> Result<FetchResult, SyncError> {
        self.inner.fetch_vault(s, t, n)
    }
    fn head_vault(&self, s: &str, t: &str, n: Option<u64>) -> Result<FetchResult, SyncError> {
        self.inner.head_vault(s, t, n)
    }
    fn put_vault(
        &self,
        s: &str,
        t: &str,
        body: &[u8],
        v: Option<u64>,
        timeout: Option<Duration>,
    ) -> Result<SlotState, SyncError> {
        thread::sleep(self.put_delay);
        self.inner.put_vault(s, t, body, v, timeout)
    }
    fn rotate_token(&self, s: &str, t: &str) -> Result<String, SyncError> {
        self.inner.rotate_token(s, t)
    }
    fn delete_vault(&self, s: &str, t: &str) -> Result<(), SyncError> {
        self.inner.delete_vault(s, t)
    }
    fn list_versions(&self, s: &str, t: &str) -> Result<Vec<VersionInfo>, SyncError> {
        self.inner.list_versions(s, t)
    }
    fn fetch_version(&self, s: &str, t: &str, v: u64) -> Result<(Vec<u8>, SlotState), SyncError> {
        self.inner.fetch_version(s, t, v)
    }
}

struct Host2 {
    path: PathBuf,
    unlocked: Unlocked,
    statuses: Mutex<Vec<SyncStatus>>,
}

impl VaultHost for Host2 {
    fn vault_path(&self) -> PathBuf {
        self.path.clone()
    }
    fn unlocked(&self) -> Option<Unlocked> {
        Some(self.unlocked.clone())
    }
    fn on_remote_adopted(&self, _: &AdoptReport) {}
    fn on_status(&self, status: &SyncStatus) {
        self.statuses.lock().unwrap().push(status.clone());
    }
}

struct Rig {
    dir: TempDir,
    file: VaultFile,
    unlocked: Unlocked,
    inner: Arc<FakeTransport>,
    handle: SyncHandle,
}

fn fast() -> CoordinatorConfig {
    CoordinatorConfig {
        debounce: Duration::from_millis(40),
        poll_interval: Duration::from_millis(60),
        backoff: vec![Duration::from_millis(60)],
        switch_push_timeout: Duration::from_secs(3),
    }
}

fn rig(put_delay: Duration, with_profile: bool) -> Rig {
    let dir = TempDir::new().unwrap();
    let file = VaultFile::new(dir.path().join("passwords.maine"));
    let unlocked = file.create("runner master password").unwrap();
    let host = Arc::new(Host2 {
        path: file.path().to_path_buf(),
        unlocked: unlocked.clone(),
        statuses: Mutex::new(Vec::new()),
    });
    let mut store =
        SyncProfileStore::open(dir.path().join("state"), Arc::new(MemoryTokenStore::new()))
            .unwrap();
    if with_profile {
        let mut profile = SyncProfile::new("Homelab", "http://127.0.0.1:1");
        profile.last_version = 1;
        profile.last_sha256 = Some(sha256_hex(&std::fs::read(file.path()).unwrap()));
        store.add(profile, "maine-TOK").unwrap();
    }
    let inner = Arc::new(FakeTransport::new());
    let transport: Arc<dyn Transport> = Arc::new(SlowTransport {
        inner: inner.clone(),
        put_delay,
    });
    let coordinator = Coordinator::new(
        transport,
        Arc::new(Mutex::new(store)),
        host.clone(),
        Arc::new(SystemClock),
        fast(),
    );
    Rig {
        dir,
        file,
        unlocked,
        inner,
        handle: SyncHandle::start(coordinator),
    }
}

impl Rig {
    fn edit(&self) {
        self.file
            .update(&self.unlocked, |c| {
                c.hosts.push(Host::new("a", "a.example.com"))
            })
            .unwrap();
    }

    fn put_ok(&self, version: u64) -> Result<SlotState, SyncError> {
        let bytes = std::fs::read(self.file.path()).unwrap();
        Ok(SlotState::with(
            version,
            bytes.len() as u64,
            sha256_hex(&bytes),
        ))
    }
}

fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

fn count(rig: &Rig, name: &str) -> usize {
    rig.inner
        .call_names()
        .iter()
        .filter(|n| **n == name)
        .count()
}

#[test]
fn a_local_write_is_pushed_after_the_debounce() {
    let rig = rig(Duration::ZERO, true);
    // Polling runs every 60 ms; give every poll an answer so the status
    // cannot flip to a failure mid-test.
    for _ in 0..200 {
        rig.inner
            .enqueue_fetch(Ok(FetchResult::NotModified(SlotState::new(1))));
    }
    rig.handle.unlocked();
    wait_until("the unlock pull", || count(&rig, "fetch") == 1);
    rig.edit();
    rig.inner.enqueue_put(rig.put_ok(2));

    rig.handle.notify_local_write();

    wait_until("the push", || count(&rig, "put") == 1);
    wait_until("the status to settle", || {
        rig.handle.status() == SyncStatus::Idle
    });
    assert_eq!(rig.handle.active_profile().unwrap().last_version, 2);
}

#[test]
fn unlocking_pulls_and_then_polls_with_head() {
    let rig = rig(Duration::ZERO, true);
    for _ in 0..8 {
        rig.inner
            .enqueue_fetch(Ok(FetchResult::NotModified(SlotState::new(1))));
    }

    rig.handle.unlocked();

    wait_until("the pull and two polls", || {
        count(&rig, "fetch") == 1 && count(&rig, "head") >= 2
    });
    rig.handle.locked();
    thread::sleep(Duration::from_millis(100));
    let heads = count(&rig, "head");
    thread::sleep(Duration::from_millis(250));
    assert_eq!(count(&rig, "head"), heads, "locking stops the poll");
}

#[test]
fn locking_flushes_unsaved_changes() {
    let rig = rig(Duration::ZERO, true);
    // Polling runs every 60 ms; give every poll an answer so the status
    // cannot flip to a failure mid-test.
    for _ in 0..200 {
        rig.inner
            .enqueue_fetch(Ok(FetchResult::NotModified(SlotState::new(1))));
    }
    rig.handle.unlocked();
    wait_until("unlock", || rig.handle.is_unlocked());
    rig.edit();
    rig.inner.enqueue_put(rig.put_ok(2));

    rig.handle.locked();

    wait_until("the flush", || count(&rig, "put") == 1);
}

#[test]
fn quit_flush_waits_for_the_push_and_reports_a_timeout() {
    let rig = rig(Duration::from_millis(600), true);
    rig.handle.unlocked();
    // Polling runs every 60 ms; give every poll an answer so the status
    // cannot flip to a failure mid-test.
    for _ in 0..200 {
        rig.inner
            .enqueue_fetch(Ok(FetchResult::NotModified(SlotState::new(1))));
    }
    wait_until("unlock", || rig.handle.is_unlocked());
    wait_until("the unlock pull", || count(&rig, "fetch") == 1);
    assert!(
        rig.handle.quit_flush(Duration::from_secs(1)),
        "nothing to push is done at once"
    );
    assert!(!rig.handle.wants_flush_before_quit());

    rig.edit();
    assert!(rig.handle.wants_flush_before_quit());
    rig.inner.enqueue_put(rig.put_ok(2));
    let started = Instant::now();
    // The slow server takes 600 ms; give up after 100 ms.
    assert!(!rig.handle.quit_flush(Duration::from_millis(100)));
    assert!(started.elapsed() < Duration::from_millis(550));
    wait_until("the slow push to land", || count(&rig, "put") == 1);

    rig.edit();
    rig.inner.enqueue_put(rig.put_ok(3));
    assert!(rig.handle.quit_flush(Duration::from_secs(3)));
    assert_eq!(count(&rig, "put"), 2);
}

#[test]
fn status_never_waits_for_a_request_in_flight() {
    let rig = rig(Duration::from_millis(500), true);
    for _ in 0..40 {
        rig.inner
            .enqueue_fetch(Ok(FetchResult::NotModified(SlotState::new(1))));
    }
    rig.handle.unlocked();
    wait_until("unlock", || rig.handle.is_unlocked());
    wait_until("the unlock pull", || count(&rig, "fetch") == 1);
    rig.edit();
    rig.inner.enqueue_put(rig.put_ok(2));
    rig.handle.notify_local_write();
    wait_until("the push to start", || {
        rig.handle.status() == SyncStatus::Syncing
    });

    let started = Instant::now();
    assert_eq!(rig.handle.status(), SyncStatus::Syncing);
    let _ = rig.handle.profiles();
    let _ = rig.handle.wants_flush_before_quit();
    assert!(started.elapsed() < Duration::from_millis(200));
    wait_until("the push to finish", || {
        rig.handle.status() == SyncStatus::Idle
    });
}

#[test]
fn pairing_through_the_handle_starts_the_schedule() {
    let rig = rig(Duration::ZERO, false);
    rig.handle.unlocked();
    wait_until("unlock", || rig.handle.is_unlocked());
    let bytes = std::fs::read(rig.file.path()).unwrap();
    rig.inner.enqueue_create(Ok((
        "maine-NEW".into(),
        SlotState::with(1, bytes.len() as u64, sha256_hex(&bytes)),
    )));
    for _ in 0..4 {
        rig.inner
            .enqueue_fetch(Ok(FetchResult::NotModified(SlotState::new(1))));
    }

    let created = rig
        .handle
        .create_profile("127.0.0.1:1", "Mine", None)
        .unwrap();

    assert_eq!(created.profile.name, "Mine");
    wait_until("polling after create", || count(&rig, "head") >= 1);
    assert_eq!(rig.handle.profiles().len(), 1);
    assert!(rig
        .handle
        .rename_profile(created.profile.id, "  Renamed ")
        .unwrap());
    assert_eq!(rig.handle.active_profile().unwrap().name, "Renamed");
    assert!(!rig
        .handle
        .rename_profile(created.profile.id, "   ")
        .unwrap());
}

#[test]
fn shutdown_stops_the_thread_and_later_notifications_are_harmless() {
    let rig = rig(Duration::ZERO, true);
    rig.handle.shutdown();

    rig.handle.notify_local_write();
    rig.handle.unlocked();
    assert!(!rig.handle.quit_flush(Duration::from_millis(50)));
    assert!(rig.inner.calls().is_empty());
    drop(rig.dir);
}
