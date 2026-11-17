//! End to end against the real Rust sync server. Uses `MAINE_SYNC_SERVER_BIN`,
//! else `apps/sync-server/target/release/maine-sync-server`; with neither the
//! tests print a note and pass without running (build the server with
//! `cargo build --release` in `apps/sync-server` to enable them).

use maine_core::model::Host;
use maine_core::vault::{Unlocked, VaultFile};
use maine_sync::engine::{JoinPreparation, PullResult, PushResult};
use maine_sync::testing::ManualClock;
use maine_sync::token::display_token;
use maine_sync::{
    AdoptReport, Coordinator, CoordinatorConfig, FetchResult, HttpTransport, JoinOutcome,
    MemoryTokenStore, SharedProfiles, SyncEngine, SyncError, SyncProfileStore, SyncStatus,
    Transport, VaultHost,
};
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

struct Server {
    child: Child,
    url: String,
    _dir: TempDir,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("MAINE_SYNC_SERVER_BIN") {
        return Some(PathBuf::from(path));
    }
    let fallback = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../sync-server/target/release/maine-sync-server");
    fallback.exists().then_some(fallback)
}

fn start(extra: &[&str]) -> Option<Server> {
    let Some(binary) = binary() else {
        eprintln!("note: no maine-sync-server binary; skipping (set MAINE_SYNC_SERVER_BIN)");
        return None;
    };
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let dir = TempDir::new().unwrap();
    let child = Command::new(binary)
        .args(["--bind", &format!("127.0.0.1:{port}"), "--data-dir"])
        .arg(dir.path().join("server"))
        .args(extra)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the server binary must start");
    let server = Server {
        child,
        url: format!("http://127.0.0.1:{port}"),
        _dir: dir,
    };
    let transport = HttpTransport::with_request_timeout(Duration::from_secs(1));
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if transport.health(&server.url).is_ok() {
            return Some(server);
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("the sync server did not start in time");
}

struct Device {
    _dir: TempDir,
    file: VaultFile,
    unlocked: Unlocked,
}

fn device(password: &str) -> Device {
    let dir = TempDir::new().unwrap();
    let file = VaultFile::new(dir.path().join("passwords.maine"));
    let unlocked = file.create(password).unwrap();
    Device {
        _dir: dir,
        file,
        unlocked,
    }
}

fn bytes(device: &Device) -> Vec<u8> {
    fs::read(device.file.path()).unwrap()
}

fn engine(device: &Device) -> SyncEngine<HttpTransport> {
    SyncEngine::new(device.file.path(), HttpTransport::new())
}

fn add_host(device: &Device, name: &str) -> Host {
    add_host_with(&device.file, &device.unlocked, name)
}

fn add_host_with(file: &VaultFile, unlocked: &Unlocked, name: &str) -> Host {
    let host = Host::new(name, format!("{name}.example.com"));
    let copy = host.clone();
    file.update(unlocked, move |c| c.hosts.push(copy)).unwrap();
    host
}

const MASTER: &str = "integration master password";

/// Port of `SyncServerIntegrationTests.createJoinPushPullConflictRotateDelete`.
#[test]
fn create_join_push_pull_conflict_rotate_delete() {
    let Some(server) = start(&[]) else { return };

    // Device A creates the slot from its vault; the token comes back once.
    let a = device(MASTER);
    let engine_a = engine(&a);
    let (token, created) = engine_a.create(&server.url, None).unwrap();
    assert_eq!(created.version, 1);
    assert!(token.starts_with("maine-"));

    // Device B joins with the same master password; the bytes match.
    let b = device("bootstrap different password");
    let engine_b = engine(&b);
    match engine_b.prepare_join(&server.url, &token, MASTER).unwrap() {
        JoinPreparation::EmptySlot => panic!("the slot should not be empty"),
        JoinPreparation::Ready(remote, state) => {
            engine_b.adopt_prepared_join(&remote, &state).unwrap()
        }
    }
    assert_eq!(bytes(&b), bytes(&a));
    // B unlocks the adopted vault, exactly like the join flow does.
    let b_unlocked = b.file.unlock(MASTER).unwrap();

    // A edits and pushes; B pulls the exact bytes.
    add_host(&a, "sync");
    let pushed = engine_a
        .push(&server.url, &token, 1, created.sha256.as_deref(), None)
        .unwrap();
    let pushed_state = match pushed {
        PushResult::Stored(state) => state,
        other => panic!("expected the push to be stored, got {other:?}"),
    };
    assert_eq!(pushed_state.version, 2);

    match engine_b
        .pull(&server.url, &token, 1, false, &b_unlocked)
        .unwrap()
    {
        PullResult::Adopted {
            state,
            conflict_copy,
        } => {
            assert_eq!(state.version, 2);
            assert_eq!(conflict_copy, None);
        }
        other => panic!("expected the pull to adopt version 2, got {other:?}"),
    }
    assert_eq!(bytes(&b), bytes(&a));

    // A second pull is a 304.
    assert!(matches!(
        engine_b
            .pull(&server.url, &token, 2, false, &b_unlocked)
            .unwrap(),
        PullResult::NotModified(_)
    ));

    // A stale write is a conflict the client can see and reconcile.
    assert_eq!(
        engine_a
            .push(&server.url, &token, 1, Some("stale"), None)
            .unwrap_err(),
        SyncError::Conflict { current_version: 2 }
    );

    // Any display form of the token is accepted by the server.
    let display = display_token(&token).unwrap();
    assert!(engine_a.head(&server.url, &display, 2).is_ok());
    assert!(engine_a
        .head(&server.url, &display.to_lowercase().replace('-', " "), 2)
        .is_ok());

    // History is kept; an old version can be saved as a copy.
    let versions = engine_a.list_versions(&server.url, &token).unwrap();
    assert_eq!(
        versions.iter().map(|v| v.version).collect::<Vec<_>>(),
        vec![2, 1]
    );
    let copy = engine_a.save_version_copy(&server.url, &token, 1).unwrap();
    assert!(copy.starts_with("passwords.remote-"));

    // Rotation kills the old token immediately.
    let rotated = engine_a.rotate(&server.url, &token).unwrap();
    assert_ne!(rotated, token);
    assert_eq!(
        engine_a.head(&server.url, &token, 2).unwrap_err(),
        SyncError::Unauthorized
    );
    assert!(engine_a.head(&server.url, &rotated, 2).is_ok());

    // Deletion wipes the slot; the new token stops working too, and the local
    // vault still opens.
    engine_a.delete(&server.url, &rotated).unwrap();
    assert_eq!(
        engine_a.head(&server.url, &rotated, 2).unwrap_err(),
        SyncError::Unauthorized
    );
    assert!(a.file.unlock(MASTER).is_ok());
}

#[test]
fn joining_with_the_wrong_master_password_changes_nothing() {
    let Some(server) = start(&[]) else { return };
    let a = device(MASTER);
    let (token, _) = engine(&a).create(&server.url, None).unwrap();
    let b = device("bootstrap different password");
    let before = bytes(&b);

    let error = engine(&b)
        .prepare_join(&server.url, &token, "definitely the wrong one")
        .unwrap_err();

    let copy = match error {
        SyncError::WrongMasterPassword { remote_copy } => remote_copy,
        other => panic!("unexpected error {other:?}"),
    };
    assert_eq!(bytes(&b), before);
    assert_eq!(
        fs::read(b.file.path().parent().unwrap().join(copy)).unwrap(),
        bytes(&a),
        "the downloaded bytes are kept"
    );
}

#[test]
fn an_empty_slot_is_filled_with_if_none_match() {
    let Some(server) = start(&[]) else { return };
    let transport = HttpTransport::new();
    let (token, state) = transport.create_vault(&server.url, None, b"").unwrap();
    assert_eq!(state.version, 0);
    let a = device(MASTER);
    let engine_a = engine(&a);

    assert_eq!(
        engine_a.prepare_join(&server.url, &token, MASTER).unwrap(),
        JoinPreparation::EmptySlot
    );
    // `fetch` on an empty slot is a 204 even with a precondition.
    assert!(matches!(
        transport.fetch_vault(&server.url, &token, Some(3)).unwrap(),
        FetchResult::Empty(_)
    ));
    match engine_a.push(&server.url, &token, 0, None, None).unwrap() {
        PushResult::Stored(state) => assert_eq!(state.version, 1),
        other => panic!("expected a stored push, got {other:?}"),
    }
    // The second first-write loses (412 becomes a conflict).
    assert!(matches!(
        engine_a
            .push(&server.url, &token, 0, Some("x"), None)
            .unwrap_err(),
        SyncError::Conflict { .. }
    ));
}

#[test]
fn an_oversized_vault_is_refused_with_413_and_nothing_is_stored() {
    let Some(server) = start(&["--max-blob", "256"]) else {
        return;
    };
    let a = device(MASTER);
    assert!(bytes(&a).len() > 256);

    let error = engine(&a).create(&server.url, None).unwrap_err();

    assert_eq!(error, SyncError::TooLarge);
    assert!(
        a.file.unlock(MASTER).is_ok(),
        "editing and unlocking still work"
    );
}

#[test]
fn the_create_secret_is_sent_only_when_given() {
    let Some(server) = start(&["--create-secret", "open sesame"]) else {
        return;
    };
    let a = device(MASTER);
    let engine_a = engine(&a);

    assert!(matches!(
        engine_a.create(&server.url, None).unwrap_err(),
        SyncError::Forbidden(_)
    ));
    assert!(matches!(
        engine_a.create(&server.url, Some("wrong")).unwrap_err(),
        SyncError::Forbidden(_)
    ));
    let (_, state) = engine_a.create(&server.url, Some("open sesame")).unwrap();
    assert_eq!(state.version, 1);
}

#[test]
fn a_stopped_server_is_offline_not_a_crash() {
    let Some(server) = start(&[]) else { return };
    let url = server.url.clone();
    drop(server);

    let error = HttpTransport::with_request_timeout(Duration::from_secs(2))
        .fetch_vault(&url, "maine-TOKEN", None)
        .unwrap_err();

    assert!(matches!(error, SyncError::Offline(_)), "{error:?}");
}

// MARK: - Coordinator, two devices, the real server

struct TestHost {
    path: PathBuf,
    unlocked: Mutex<Option<Unlocked>>,
    adopted: Mutex<Vec<AdoptReport>>,
    alerts: Mutex<Vec<String>>,
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
    fn on_status(&self, _status: &SyncStatus) {}
    fn on_alert(&self, title: &str, _message: &str) {
        self.alerts.lock().unwrap().push(title.to_string());
    }
}

struct Node {
    device: Device,
    host: Arc<TestHost>,
    clock: Arc<ManualClock>,
    profiles: SharedProfiles,
    coordinator: Coordinator,
}

fn node(device: Device) -> Node {
    let host = Arc::new(TestHost {
        path: device.file.path().to_path_buf(),
        unlocked: Mutex::new(Some(device.unlocked.clone())),
        adopted: Mutex::new(Vec::new()),
        alerts: Mutex::new(Vec::new()),
    });
    let clock = Arc::new(ManualClock::new());
    let state_dir = device.file.path().parent().unwrap().join("state");
    let profiles: SharedProfiles = Arc::new(Mutex::new(
        SyncProfileStore::open(state_dir, Arc::new(MemoryTokenStore::new())).unwrap(),
    ));
    let coordinator = Coordinator::new(
        Arc::new(HttpTransport::new()),
        profiles.clone(),
        host.clone(),
        clock.clone(),
        CoordinatorConfig::default(),
    );
    Node {
        device,
        host,
        clock,
        profiles,
        coordinator,
    }
}

impl Node {
    fn advance(&mut self, seconds: u64) {
        self.clock.advance(Duration::from_secs(seconds));
        self.coordinator.tick();
    }

    fn version(&self) -> u64 {
        self.profiles
            .lock()
            .unwrap()
            .active_profile()
            .unwrap()
            .last_version
    }
}

#[test]
fn two_coordinators_sync_edit_conflict_and_resolve_through_the_real_server() {
    let Some(server) = start(&[]) else { return };

    // A creates the slot; B joins with the master password and unlocks.
    let mut a = node(device(MASTER));
    let created = a
        .coordinator
        .create_profile(&server.url, "A", None)
        .unwrap();
    a.coordinator.vault_did_unlock();
    assert_eq!(a.version(), 1);

    let mut b = node(device("bootstrap different password"));
    let joined = b
        .coordinator
        .join_profile(&server.url, &created.token, "B", MASTER)
        .unwrap();
    assert!(matches!(joined, JoinOutcome::Joined(_)));
    assert_eq!(bytes(&b.device), bytes(&a.device));
    *b.host.unlocked.lock().unwrap() = Some(b.device.file.unlock(MASTER).unwrap());
    b.coordinator.vault_did_unlock();
    assert_eq!(b.coordinator.status(), SyncStatus::Idle);

    // A edits; after the debounce the vault is pushed; B's next poll adopts it.
    let kept = add_host(&a.device, "kept");
    a.coordinator.local_vault_changed();
    a.advance(1);
    assert_eq!(a.version(), 1, "not before the debounce");
    a.advance(1);
    assert_eq!(a.version(), 2);
    b.advance(30);
    assert_eq!(b.version(), 2);
    assert_eq!(bytes(&b.device), bytes(&a.device));
    let report = b.host.adopted.lock().unwrap().last().cloned().unwrap();
    assert!(report.hosts_changed);
    assert_eq!(report.conflict_copy, None);
    assert!(report.removed_host_ids.is_empty());

    // Both edit from version 2. A wins the race; B conflicts, keeps its own
    // version as a copy and follows the server.
    add_host(&a.device, "from-a");
    a.coordinator.local_vault_changed();
    a.advance(2);
    assert_eq!(a.version(), 3);
    // B's `device.unlocked` belongs to the bootstrap vault it replaced; the
    // adopted vault's key is the one the host holds.
    let b_key = b.host.unlocked.lock().unwrap().clone().unwrap();
    let b_edit = add_host_with(&b.device.file, &b_key, "from-b");
    let b_bytes = bytes(&b.device);
    b.coordinator.local_vault_changed();
    b.advance(2);

    assert_eq!(bytes(&b.device), bytes(&a.device), "B follows the server");
    assert_eq!(b.version(), 3);
    let report = b.host.adopted.lock().unwrap().last().cloned().unwrap();
    let copy = report.conflict_copy.expect("a conflict copy");
    assert_eq!(fs::read(copy).unwrap(), b_bytes);
    assert!(report.removed_host_ids.contains(&b_edit.id));
    assert!(!report.removed_host_ids.contains(&kept.id));
    assert!(matches!(
        b.coordinator.status(),
        SyncStatus::Conflicted { .. }
    ));
    assert_eq!(
        *b.host.alerts.lock().unwrap(),
        vec!["Another device changed the vault"]
    );

    // Everything is quiet afterwards: a poll on each side is a 304.
    a.advance(30);
    b.advance(30);
    assert_eq!(a.coordinator.status(), SyncStatus::Idle);
    assert_eq!(b.coordinator.status(), SyncStatus::Idle);

    // A rotates; its stored token is replaced; B is rejected on its next poll.
    let a_id = a.profiles.lock().unwrap().active_profile_id().unwrap();
    let rotated = a.coordinator.rotate_token(a_id).unwrap();
    assert_ne!(rotated, created.token);
    b.advance(30);
    assert_eq!(b.coordinator.status(), SyncStatus::TokenRejected);
    a.advance(30);
    assert_eq!(a.coordinator.status(), SyncStatus::Idle);

    // A deletes the slot and forgets the profile; the local vault still opens.
    a.coordinator.delete_remote_data(a_id).unwrap();
    assert!(a.profiles.lock().unwrap().active_profile().is_none());
    assert_eq!(a.coordinator.status(), SyncStatus::Off);
    assert!(a.device.file.unlock(MASTER).is_ok());
}

#[test]
fn a_vault_edited_while_offline_is_pushed_once_the_server_is_back() {
    let Some(server) = start(&[]) else { return };
    let mut a = node(device(MASTER));
    let created = a
        .coordinator
        .create_profile(&server.url, "A", None)
        .unwrap();
    a.coordinator.vault_did_unlock();
    let id = created.profile.id;

    // Point the profile at a dead port, edit, let the push fail.
    let live_url = server.url.clone();
    a.profiles
        .lock()
        .unwrap()
        .update(id, |p| p.server_url = "http://127.0.0.1:1".into())
        .unwrap();
    add_host(&a.device, "offline edit");
    a.coordinator.local_vault_changed();
    a.advance(2);
    assert!(matches!(a.coordinator.status(), SyncStatus::Offline(_)));
    assert_eq!(a.version(), 1);

    // The server comes back (the profile points at it again): the next poll
    // after the back-off finds nothing new remotely, and a write pushes.
    a.profiles
        .lock()
        .unwrap()
        .update(id, |p| p.server_url = live_url)
        .unwrap();
    a.coordinator.local_vault_changed();
    a.advance(2);
    assert_eq!(a.version(), 2);
    assert_eq!(a.coordinator.status(), SyncStatus::Idle);
}
