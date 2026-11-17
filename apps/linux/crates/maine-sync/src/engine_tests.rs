use super::*;
use crate::testing::{Call, FakeTransport};
use std::sync::Arc;
use tempfile::TempDir;

const SERVER: &str = "http://127.0.0.1:8484";

fn make_vault(dir: &Path, password: &str) -> (VaultFile, Unlocked) {
    let file = VaultFile::new(dir.join("passwords.maine"));
    let unlocked = file.create(password).unwrap();
    (file, unlocked)
}

fn names(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

fn engine(file: &VaultFile, transport: &Arc<FakeTransport>) -> SyncEngine<Arc<FakeTransport>> {
    SyncEngine::new(file.path(), transport.clone())
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn create_uploads_the_current_vault_and_returns_the_token() {
    let dir = TempDir::new().unwrap();
    let (file, _) = make_vault(dir.path(), "sync create master password");
    let transport = Arc::new(FakeTransport::new());
    let state = SlotState::with(1, 400, "abc123");
    transport.enqueue_create(Ok(("maine-TEST-TEST".into(), state.clone())));
    let engine = engine(&file, &transport);

    let (token, created) = engine.create(SERVER, Some("shh")).unwrap();

    assert_eq!(token, "maine-TEST-TEST");
    assert_eq!(created, state);
    assert_eq!(
        transport.calls(),
        vec![Call::Create {
            create_secret: Some("shh".into()),
            body: fs::read(file.path()).unwrap(),
        }]
    );
}

#[test]
fn create_without_a_local_vault_sends_an_empty_body() {
    let dir = TempDir::new().unwrap();
    let transport = Arc::new(FakeTransport::new());
    transport.enqueue_create(Ok(("maine-T".into(), SlotState::new(0))));
    let engine = SyncEngine::new(dir.path().join("passwords.maine"), transport.clone());

    engine.create(SERVER, None).unwrap();

    assert_eq!(
        transport.calls(),
        vec![Call::Create {
            create_secret: None,
            body: Vec::new()
        }]
    );
}

#[test]
fn push_skips_when_the_file_matches_the_last_synced_hash() {
    let dir = TempDir::new().unwrap();
    let (file, _) = make_vault(dir.path(), "sync push master password");
    let sha = sha256_hex(&fs::read(file.path()).unwrap());
    let transport = Arc::new(FakeTransport::new());
    let engine = engine(&file, &transport);

    let result = engine
        .push(SERVER, "maine-TEST", 3, Some(&sha), None)
        .unwrap();

    match result {
        PushResult::Skipped(state) => assert_eq!(state.version, 3),
        other => panic!("expected the push to be skipped, got {other:?}"),
    }
    assert!(transport.calls().is_empty());
}

#[test]
fn push_stores_new_bytes_and_surfaces_conflicts() {
    let dir = TempDir::new().unwrap();
    let (file, _) = make_vault(dir.path(), "sync push master password");
    let transport = Arc::new(FakeTransport::new());
    let engine = engine(&file, &transport);

    transport.enqueue_put(Ok(SlotState::with(4, 10, "def456")));
    let stored = engine
        .push(SERVER, "maine-TEST", 3, Some("old"), None)
        .unwrap();
    match stored {
        PushResult::Stored(state) => assert_eq!(state.version, 4),
        other => panic!("expected a stored push, got {other:?}"),
    }

    transport.enqueue_put(Err(SyncError::Conflict { current_version: 9 }));
    let error = engine
        .push(SERVER, "maine-TEST", 4, Some("stale"), None)
        .unwrap_err();
    assert_eq!(error, SyncError::Conflict { current_version: 9 });
}

#[test]
fn push_sends_if_match_for_a_synced_slot_and_none_for_the_first_version() {
    let dir = TempDir::new().unwrap();
    let (file, _) = make_vault(dir.path(), "sync push master password");
    let transport = Arc::new(FakeTransport::new());
    let engine = engine(&file, &transport);
    let bytes = fs::read(file.path()).unwrap();

    transport.enqueue_put(Ok(SlotState::new(1)));
    engine.push(SERVER, "t", 0, None, None).unwrap();
    transport.enqueue_put(Ok(SlotState::new(6)));
    engine
        .push(SERVER, "t", 5, Some("old"), Some(Duration::from_secs(3)))
        .unwrap();

    assert_eq!(
        transport.calls(),
        vec![
            Call::Put {
                token: "t".into(),
                body: bytes.clone(),
                expected_version: None,
                timeout: None
            },
            Call::Put {
                token: "t".into(),
                body: bytes,
                expected_version: Some(5),
                timeout: Some(Duration::from_secs(3))
            },
        ]
    );
}

#[test]
fn push_without_a_local_vault_is_refused_before_any_request() {
    let dir = TempDir::new().unwrap();
    let transport = Arc::new(FakeTransport::new());
    let engine = SyncEngine::new(dir.path().join("passwords.maine"), transport.clone());

    assert_eq!(
        engine.push(SERVER, "t", 0, None, None).unwrap_err(),
        SyncError::NoLocalVault
    );
    assert!(transport.calls().is_empty());
}

#[test]
fn join_with_the_wrong_master_password_changes_nothing() {
    let local_dir = TempDir::new().unwrap();
    let remote_dir = TempDir::new().unwrap();
    let (local, _) = make_vault(local_dir.path(), "local sync master password");
    let (remote, _) = make_vault(remote_dir.path(), "remote sync master password");
    let local_bytes = fs::read(local.path()).unwrap();
    let remote_bytes = fs::read(remote.path()).unwrap();
    let state = SlotState::with(2, remote_bytes.len() as u64, sha256_hex(&remote_bytes));

    let transport = Arc::new(FakeTransport::new());
    transport.enqueue_fetch(Ok(FetchResult::Blob(remote_bytes.clone(), state)));
    let engine = engine(&local, &transport);

    let error = engine
        .prepare_join(SERVER, "maine-TEST", "definitely wrong")
        .unwrap_err();
    let copy = match error {
        SyncError::WrongMasterPassword { remote_copy } => remote_copy,
        other => panic!("unexpected error {other:?}"),
    };

    assert_eq!(fs::read(local.path()).unwrap(), local_bytes);
    assert!(copy.starts_with("passwords.remote-"));
    assert_eq!(
        fs::read(local_dir.path().join(&copy)).unwrap(),
        remote_bytes
    );
    assert_eq!(mode(&local_dir.path().join(&copy)), 0o600);
}

#[test]
fn join_into_an_empty_slot_offers_to_push() {
    let dir = TempDir::new().unwrap();
    let (file, _) = make_vault(dir.path(), "sync empty slot master");
    let transport = Arc::new(FakeTransport::new());
    transport.enqueue_fetch(Ok(FetchResult::Empty(SlotState::new(0))));
    let engine = engine(&file, &transport);

    let preparation = engine
        .prepare_join(SERVER, "maine-TEST", "whatever")
        .unwrap();

    assert_eq!(preparation, JoinPreparation::EmptySlot);
    assert_eq!(
        transport.calls(),
        vec![Call::Fetch {
            token: "maine-TEST".into(),
            if_none_match: None
        }]
    );
}

#[test]
fn join_accepts_a_vault_the_master_password_opens() {
    let local_dir = TempDir::new().unwrap();
    let remote_dir = TempDir::new().unwrap();
    let (local, _) = make_vault(local_dir.path(), "local sync master password");
    let (remote, _) = make_vault(remote_dir.path(), "remote sync master password");
    let remote_bytes = fs::read(remote.path()).unwrap();
    let state = SlotState::with(2, remote_bytes.len() as u64, sha256_hex(&remote_bytes));
    let transport = Arc::new(FakeTransport::new());
    transport.enqueue_fetch(Ok(FetchResult::Blob(remote_bytes.clone(), state.clone())));
    let engine = engine(&local, &transport);

    let preparation = engine
        .prepare_join(SERVER, "maine-TEST", "remote sync master password")
        .unwrap();

    assert_eq!(preparation, JoinPreparation::Ready(remote_bytes, state));
    // Preparing never touches the live vault or leaves a remote copy.
    assert_eq!(names(local_dir.path()), vec!["passwords.maine".to_string()]);
}

#[test]
fn join_keeps_an_unreadable_download_and_says_so() {
    let dir = TempDir::new().unwrap();
    let (file, _) = make_vault(dir.path(), "local sync master password");
    let junk = b"not a vault at all".to_vec();
    let state = SlotState::with(2, junk.len() as u64, sha256_hex(&junk));
    let transport = Arc::new(FakeTransport::new());
    transport.enqueue_fetch(Ok(FetchResult::Blob(junk.clone(), state)));
    let engine = engine(&file, &transport);

    let error = engine
        .prepare_join(SERVER, "maine-TEST", "local sync master password")
        .unwrap_err();

    let copy = match error {
        SyncError::UnreadableRemoteVault { remote_copy } => remote_copy,
        other => panic!("unexpected error {other:?}"),
    };
    assert_eq!(fs::read(dir.path().join(copy)).unwrap(), junk);
}

#[test]
fn a_checksum_mismatch_keeps_the_bytes_and_adopts_nothing() {
    let dir = TempDir::new().unwrap();
    let (file, unlocked) = make_vault(dir.path(), "sync checksum master");
    let before = fs::read(file.path()).unwrap();
    let bytes = before.clone();
    let transport = Arc::new(FakeTransport::new());
    let wrong = SlotState::with(5, bytes.len() as u64, "0".repeat(64));
    transport.enqueue_fetch(Ok(FetchResult::Blob(bytes.clone(), wrong.clone())));
    transport.enqueue_fetch(Ok(FetchResult::Blob(bytes.clone(), wrong)));
    let engine = engine(&file, &transport);

    // Pull and join both verify the checksum before anything else.
    let pull = engine.pull(SERVER, "t", 1, true, &unlocked).unwrap_err();
    assert!(matches!(pull, SyncError::ShaMismatch { .. }));
    let join = engine
        .prepare_join(SERVER, "t", "sync checksum master")
        .unwrap_err();
    assert!(matches!(join, SyncError::ShaMismatch { .. }));

    assert_eq!(fs::read(file.path()).unwrap(), before);
    let kept: Vec<_> = names(dir.path())
        .into_iter()
        .filter(|n| n.starts_with("passwords.remote-"))
        .collect();
    assert_eq!(kept.len(), 2, "{kept:?}");
    assert!(!names(dir.path())
        .iter()
        .any(|n| n.starts_with("passwords.conflict-")));
}

#[test]
fn adopting_a_join_moves_the_local_vault_to_a_pre_sync_copy() {
    let local_dir = TempDir::new().unwrap();
    let remote_dir = TempDir::new().unwrap();
    let (local, _) = make_vault(local_dir.path(), "local sync master password");
    let (remote, _) = make_vault(remote_dir.path(), "remote sync master password");
    let local_bytes = fs::read(local.path()).unwrap();
    let remote_bytes = fs::read(remote.path()).unwrap();
    let state = SlotState::with(2, remote_bytes.len() as u64, sha256_hex(&remote_bytes));
    let engine = engine(&local, &Arc::new(FakeTransport::new()));

    engine.adopt_prepared_join(&remote_bytes, &state).unwrap();

    assert_eq!(fs::read(local.path()).unwrap(), remote_bytes);
    let copy = names(local_dir.path())
        .into_iter()
        .find(|n| n.starts_with("passwords.pre-sync-"))
        .expect("a pre-sync copy");
    assert_eq!(fs::read(local_dir.path().join(&copy)).unwrap(), local_bytes);
    assert_eq!(mode(local.path()), 0o600);
    assert_eq!(mode(&local_dir.path().join(copy)), 0o600);
}

#[test]
fn adopting_a_join_without_a_local_vault_makes_no_pre_sync_copy() {
    let dir = TempDir::new().unwrap();
    let remote_dir = TempDir::new().unwrap();
    let (remote, _) = make_vault(remote_dir.path(), "remote sync master password");
    let remote_bytes = fs::read(remote.path()).unwrap();
    let engine = SyncEngine::new(
        dir.path().join("fresh").join("passwords.maine"),
        Arc::new(FakeTransport::new()),
    );

    engine
        .adopt_prepared_join(&remote_bytes, &SlotState::new(1))
        .unwrap();

    assert_eq!(fs::read(engine.vault_path()).unwrap(), remote_bytes);
    assert_eq!(
        names(&dir.path().join("fresh")),
        vec!["passwords.maine".to_string()]
    );
    assert_eq!(
        engine
            .adopt_prepared_join(&remote_bytes, &SlotState::new(0))
            .unwrap_err(),
        SyncError::Malformed("The slot reported version 0.".into())
    );
}

#[test]
fn pull_keeps_a_dirty_local_vault_as_a_conflict_copy() {
    let dir = TempDir::new().unwrap();
    let (file, unlocked) = make_vault(dir.path(), "sync pull master password");
    let original = fs::read(file.path()).unwrap();
    let edited = b"a local edit that must survive".to_vec();
    fs::write(file.path(), &edited).unwrap();
    let state = SlotState::with(7, original.len() as u64, sha256_hex(&original));
    let transport = Arc::new(FakeTransport::new());
    transport.enqueue_fetch(Ok(FetchResult::Blob(original.clone(), state)));
    let engine = engine(&file, &transport);

    let result = engine
        .pull(SERVER, "maine-TEST", 3, true, &unlocked)
        .unwrap();

    let (adopted, conflict_copy) = match result {
        PullResult::Adopted {
            state,
            conflict_copy,
        } => (state, conflict_copy),
        other => panic!("expected the remote vault to be adopted, got {other:?}"),
    };
    assert_eq!(adopted.version, 7);
    let name = conflict_copy.expect("a conflict copy");
    assert!(name.starts_with("passwords.conflict-"));
    assert_eq!(fs::read(dir.path().join(&name)).unwrap(), edited);
    assert_eq!(mode(&dir.path().join(&name)), 0o600);
    assert_eq!(fs::read(file.path()).unwrap(), original);
    assert_eq!(mode(file.path()), 0o600);
    assert_eq!(
        transport.calls(),
        vec![Call::Fetch {
            token: "maine-TEST".into(),
            if_none_match: Some(3)
        }]
    );
}

#[test]
fn pull_of_a_clean_local_vault_replaces_it_silently() {
    let dir = TempDir::new().unwrap();
    let (file, unlocked) = make_vault(dir.path(), "sync pull master password");
    let remote_dir = TempDir::new().unwrap();
    // A later version of the same vault: same file, edited through the core.
    let remote = VaultFile::new(remote_dir.path().join("passwords.maine"));
    fs::copy(file.path(), remote.path()).unwrap();
    remote
        .update(&unlocked, |c| {
            c.hosts
                .push(maine_core::model::Host::new("Remote", "remote.example.com"))
        })
        .unwrap();
    let remote_bytes = fs::read(remote.path()).unwrap();
    let state = SlotState::with(2, remote_bytes.len() as u64, sha256_hex(&remote_bytes));
    let transport = Arc::new(FakeTransport::new());
    transport.enqueue_fetch(Ok(FetchResult::Blob(remote_bytes.clone(), state)));
    let engine = engine(&file, &transport);

    let result = engine.pull(SERVER, "t", 1, false, &unlocked).unwrap();

    match result {
        PullResult::Adopted { conflict_copy, .. } => assert_eq!(conflict_copy, None),
        other => panic!("expected adoption, got {other:?}"),
    }
    assert_eq!(fs::read(file.path()).unwrap(), remote_bytes);
    assert_eq!(names(dir.path()), vec!["passwords.maine".to_string()]);
}

#[test]
fn pull_passes_through_not_modified_and_empty() {
    let dir = TempDir::new().unwrap();
    let (file, unlocked) = make_vault(dir.path(), "sync pull master password");
    let before = fs::read(file.path()).unwrap();
    let transport = Arc::new(FakeTransport::new());
    transport.enqueue_fetch(Ok(FetchResult::NotModified(SlotState::new(3))));
    transport.enqueue_fetch(Ok(FetchResult::Empty(SlotState::new(0))));
    transport.enqueue_fetch(Ok(FetchResult::Current(SlotState::new(3))));
    let engine = engine(&file, &transport);

    assert_eq!(
        engine.pull(SERVER, "t", 3, true, &unlocked).unwrap(),
        PullResult::NotModified(SlotState::new(3))
    );
    assert_eq!(
        engine.pull(SERVER, "t", 3, true, &unlocked).unwrap(),
        PullResult::Empty(SlotState::new(0))
    );
    assert!(matches!(
        engine.pull(SERVER, "t", 3, true, &unlocked).unwrap_err(),
        SyncError::Malformed(_)
    ));
    assert_eq!(fs::read(file.path()).unwrap(), before);
    assert_eq!(names(dir.path()), vec!["passwords.maine".to_string()]);
}

#[test]
fn pull_refuses_a_vault_this_session_key_cannot_open() {
    let local_dir = TempDir::new().unwrap();
    let remote_dir = TempDir::new().unwrap();
    let (local, unlocked) = make_vault(local_dir.path(), "local sync master password");
    let (remote, _) = make_vault(remote_dir.path(), "remote sync master password");
    let local_bytes = fs::read(local.path()).unwrap();
    let remote_bytes = fs::read(remote.path()).unwrap();
    let state = SlotState::with(5, remote_bytes.len() as u64, sha256_hex(&remote_bytes));
    let transport = Arc::new(FakeTransport::new());
    transport.enqueue_fetch(Ok(FetchResult::Blob(remote_bytes, state)));
    let engine = engine(&local, &transport);

    let error = engine
        .pull(SERVER, "maine-TEST", 2, false, &unlocked)
        .unwrap_err();

    let copy = match error {
        SyncError::ForeignVault { remote_copy } => remote_copy,
        other => panic!("unexpected error {other:?}"),
    };
    assert_eq!(fs::read(local.path()).unwrap(), local_bytes);
    assert!(copy.starts_with("passwords.remote-"));
    assert!(names(local_dir.path()).contains(&copy));
}

#[test]
fn rotate_and_delete_pass_through_to_the_server() {
    let dir = TempDir::new().unwrap();
    let (file, _) = make_vault(dir.path(), "sync manage master password");
    let transport = Arc::new(FakeTransport::new());
    let engine = engine(&file, &transport);

    transport.enqueue_rotate(Ok("maine-NEW1-NEW2".into()));
    let rotated = engine.rotate(SERVER, "maine-TEST").unwrap();
    assert_eq!(rotated, "maine-NEW1-NEW2");

    transport.enqueue_delete(Ok(()));
    engine.delete(SERVER, &rotated).unwrap();

    assert_eq!(transport.call_names(), vec!["rotate", "delete"]);
}

#[test]
fn head_asks_with_the_last_version() {
    let dir = TempDir::new().unwrap();
    let (file, _) = make_vault(dir.path(), "sync head master password");
    let transport = Arc::new(FakeTransport::new());
    transport.enqueue_fetch(Ok(FetchResult::NotModified(SlotState::new(4))));
    let engine = engine(&file, &transport);

    assert_eq!(
        engine.head(SERVER, "t", 4).unwrap(),
        FetchResult::NotModified(SlotState::new(4))
    );
    assert_eq!(
        transport.calls(),
        vec![Call::Head {
            token: "t".into(),
            if_none_match: Some(4)
        }]
    );
}

#[test]
fn copies_taken_in_the_same_second_never_overwrite_each_other() {
    let dir = TempDir::new().unwrap();
    let (file, _) = make_vault(dir.path(), "sync copies master password");
    let engine = SyncEngine::new(file.path(), Arc::new(FakeTransport::new()))
        .with_timestamp(|| "20260101-000000".to_string());

    let a = engine.save_copy(b"first", "conflict").unwrap();
    let b = engine.save_copy(b"second", "conflict").unwrap();

    assert_eq!(a, "passwords.conflict-20260101-000000.maine");
    assert_eq!(b, "passwords.conflict-20260101-000000-2.maine");
    assert_eq!(fs::read(dir.path().join(a)).unwrap(), b"first");
    assert_eq!(fs::read(dir.path().join(b)).unwrap(), b"second");
}

#[test]
fn workspace_files_move_and_copy_without_losing_the_vault() {
    let dir = TempDir::new().unwrap();
    let (file, _) = make_vault(dir.path(), "sync workspace master");
    let bytes = fs::read(file.path()).unwrap();
    let engine = engine(&file, &Arc::new(FakeTransport::new()));
    let id = Uuid::new_v4();
    let workspace = engine.workspace_path(id);
    assert_eq!(
        workspace.file_name().unwrap().to_string_lossy(),
        format!(
            "passwords.workspace-{}.maine",
            id.hyphenated().to_string().to_uppercase()
        )
    );

    // Copy leaves the live vault in place.
    engine.copy_local_vault(&workspace).unwrap();
    assert_eq!(fs::read(&workspace).unwrap(), bytes);
    assert!(file.exists());
    assert_eq!(mode(&workspace), 0o600);

    // Move takes it away; adopt puts it back, replacing nothing it shouldn't.
    engine.move_local_vault(&workspace).unwrap();
    assert!(!file.exists());
    engine.adopt_local_vault(&workspace).unwrap();
    assert_eq!(fs::read(file.path()).unwrap(), bytes);
    assert!(!workspace.exists());

    // Adopting a missing workspace is an error and changes nothing.
    assert_eq!(
        engine.adopt_local_vault(&workspace).unwrap_err(),
        SyncError::NoLocalVault
    );
    assert_eq!(fs::read(file.path()).unwrap(), bytes);

    // Moving aside names the copy and returns it.
    let aside = engine.move_local_vault_aside("pre-sync").unwrap().unwrap();
    assert!(aside.starts_with("passwords.pre-sync-"));
    assert!(!file.exists());
    assert_eq!(engine.move_local_vault_aside("pre-sync").unwrap(), None);
}

#[test]
fn nothing_is_left_behind_by_an_atomic_replace() {
    let dir = TempDir::new().unwrap();
    let (file, unlocked) = make_vault(dir.path(), "sync atomic master password");
    let original = fs::read(file.path()).unwrap();
    let transport = Arc::new(FakeTransport::new());
    transport.enqueue_fetch(Ok(FetchResult::Blob(
        original.clone(),
        SlotState::with(2, original.len() as u64, sha256_hex(&original)),
    )));
    let engine = engine(&file, &transport);

    engine.pull(SERVER, "t", 1, false, &unlocked).unwrap();

    assert_eq!(names(dir.path()), vec!["passwords.maine".to_string()]);
}

#[test]
fn old_versions_are_kept_as_copies_not_adopted() {
    let dir = TempDir::new().unwrap();
    let (file, _) = make_vault(dir.path(), "sync versions master");
    let before = fs::read(file.path()).unwrap();
    let old = b"old version bytes".to_vec();
    let transport = Arc::new(FakeTransport::new());
    transport.enqueue_version_blob(Ok((
        old.clone(),
        SlotState::with(2, old.len() as u64, sha256_hex(&old)),
    )));
    let engine = engine(&file, &transport);

    let name = engine.save_version_copy(SERVER, "t", 2).unwrap();

    assert!(name.starts_with("passwords.remote-"));
    assert_eq!(fs::read(dir.path().join(name)).unwrap(), old);
    assert_eq!(fs::read(file.path()).unwrap(), before);
    assert_eq!(transport.calls(), vec![Call::FetchVersion(2)]);
}

// The core contract the engine relies on (macOS `PasswordStoreSyncTests`).

#[test]
fn validate_accepts_the_vault_and_rejects_bad_input() {
    let dir = TempDir::new().unwrap();
    let (file, _) = make_vault(dir.path(), "validation test password");
    let bytes = fs::read(file.path()).unwrap();

    VaultFile::validate_blob(&bytes, "validation test password").unwrap();
    assert_eq!(
        VaultFile::validate_blob(&bytes, "definitely not the password").unwrap_err(),
        VaultError::InvalidPassword
    );
    assert_eq!(
        VaultFile::validate_blob(b"not a vault at all", "validation test password").unwrap_err(),
        VaultError::InvalidFile
    );
}

#[test]
fn the_session_key_opens_only_this_vaults_data() {
    let first_dir = TempDir::new().unwrap();
    let second_dir = TempDir::new().unwrap();
    let (first, first_key) = make_vault(first_dir.path(), "first vault master password");
    let (second, _) = make_vault(second_dir.path(), "second vault master password");
    let first_bytes = fs::read(first.path()).unwrap();
    let second_bytes = fs::read(second.path()).unwrap();

    assert!(VaultFile::key_opens(&first_bytes, &first_key));
    assert!(!VaultFile::key_opens(&second_bytes, &first_key));
    assert!(!VaultFile::key_opens(b"not a vault", &first_key));
}

#[test]
fn sha256_hex_matches_a_known_digest() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
