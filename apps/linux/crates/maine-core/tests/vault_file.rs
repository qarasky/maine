use maine_core::model::{Host, HostGroup};
use maine_core::vault::VaultFile;
use maine_core::VaultError;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

const PASSWORD: &str = "correct horse battery";

fn vault() -> (TempDir, VaultFile) {
    let dir = tempfile::tempdir().unwrap();
    let file = VaultFile::new(dir.path().join("maine").join("passwords.maine"));
    (dir, file)
}

#[test]
fn create_then_unlock_roundtrip() {
    let (_d, v) = vault();
    let created = v.create(PASSWORD).unwrap();
    let unlocked = v.unlock(PASSWORD).unwrap();
    assert_eq!(created.id, unlocked.id);
    let c = v.contents(&unlocked).unwrap();
    assert!(c.hosts.is_empty() && c.passwords.is_empty());
}

#[test]
fn create_rejects_11_char_password() {
    let (_d, v) = vault();
    assert_eq!(
        v.create("12345678901").unwrap_err(),
        VaultError::WeakPassword
    );
    assert!(!v.exists());
    // Characters, not bytes: 12 Cyrillic letters are fine.
    assert!(v.create("абвгдеёжзийк").is_ok());
}

#[test]
fn create_refuses_existing_file() {
    let (_d, v) = vault();
    v.create(PASSWORD).unwrap();
    let before = fs::read(v.path()).unwrap();
    assert_eq!(
        v.create("another long password").unwrap_err(),
        VaultError::InvalidFile
    );
    assert_eq!(fs::read(v.path()).unwrap(), before);
}

#[test]
fn wrong_password_leaves_file_byte_identical() {
    let (_d, v) = vault();
    v.create(PASSWORD).unwrap();
    let before = fs::read(v.path()).unwrap();
    assert_eq!(
        v.unlock("not the password").unwrap_err(),
        VaultError::InvalidPassword
    );
    assert_eq!(fs::read(v.path()).unwrap(), before);
}

#[test]
fn truncated_file_is_invalid_and_untouched() {
    let (_d, v) = vault();
    v.create(PASSWORD).unwrap();
    let mut data = fs::read(v.path()).unwrap();
    data.truncate(data.len() / 2);
    fs::write(v.path(), &data).unwrap();
    assert_eq!(v.unlock(PASSWORD).unwrap_err(), VaultError::InvalidFile);
    assert_eq!(fs::read(v.path()).unwrap(), data);
}

#[test]
fn garbage_json_is_invalid_and_untouched() {
    let (_d, v) = vault();
    fs::create_dir_all(v.path().parent().unwrap()).unwrap();
    for garbage in [&b"not json"[..], b"{}", b"[]", b"{\"version\":2}", b""] {
        fs::write(v.path(), garbage).unwrap();
        assert_eq!(
            v.unlock(PASSWORD).unwrap_err(),
            VaultError::InvalidFile,
            "{garbage:?}"
        );
        assert_eq!(fs::read(v.path()).unwrap(), garbage);
    }
}

#[test]
fn version_1_is_reported_as_unsupported() {
    let (_d, v) = vault();
    fs::create_dir_all(v.path().parent().unwrap()).unwrap();
    fs::write(
        v.path(),
        br#"{"version":1,"id":"6AA6118A-7CB3-4F6E-BBED-00F0E2972F2F","salt":"AA==","iterations":600000,"verification":"AA==","passwords":{}}"#,
    )
    .unwrap();
    assert_eq!(
        v.unlock(PASSWORD).unwrap_err(),
        VaultError::UnsupportedVersion(1)
    );
}

#[test]
fn unlock_with_cyrillic_password_roundtrips() {
    let (_d, v) = vault();
    v.create("пароль-хранилища-42").unwrap();
    assert!(v.unlock("пароль-хранилища-42").is_ok());
    assert_eq!(
        v.unlock("пароль-хранилища-43").unwrap_err(),
        VaultError::InvalidPassword
    );
}

#[test]
fn nfc_and_nfd_passwords_are_different_passwords() {
    let (_d, v) = vault();
    v.create("caf\u{e9}-caf\u{e9}-caf\u{e9}").unwrap();
    assert_eq!(
        v.unlock("cafe\u{301}-cafe\u{301}-cafe\u{301}").unwrap_err(),
        VaultError::InvalidPassword
    );
}

#[test]
fn file_mode_is_0600_and_dir_0700() {
    let (_d, v) = vault();
    v.create(PASSWORD).unwrap();
    let file_mode = fs::metadata(v.path()).unwrap().permissions().mode() & 0o777;
    let dir_mode = fs::metadata(v.path().parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(file_mode, 0o600);
    assert_eq!(dir_mode, 0o700);
}

#[test]
fn update_persists_hosts_groups_and_passwords() {
    let (_d, v) = vault();
    let u = v.create(PASSWORD).unwrap();
    let group = HostGroup::new("Prod");
    let mut host = Host::new("Web", "example.com");
    host.group_id = Some(group.id);
    let id = host.id;
    v.update(&u, |c| {
        c.groups = Some(vec![group.clone()]);
        c.hosts.push(host.clone());
        c.passwords.insert(id, "s3cret".into());
    })
    .unwrap();

    let again = VaultFile::new(v.path()).unlock(PASSWORD).unwrap();
    let c = v.contents(&again).unwrap();
    assert_eq!(c.groups, Some(vec![group]));
    assert_eq!(c.hosts, vec![host]);
    assert_eq!(c.passwords.get(&id).map(String::as_str), Some("s3cret"));
}

#[test]
fn update_clears_legacy_sessions_and_blank_password_removes_entry() {
    let (_d, v) = vault();
    let u = v.create(PASSWORD).unwrap();
    let id = uuid::Uuid::new_v4();
    v.update(&u, |c| {
        c.passwords.insert(id, "x".into());
        c.sessions = Some(vec![]);
    })
    .unwrap();
    v.update(&u, |c| {
        c.passwords.remove(&id);
    })
    .unwrap();
    let c = v.contents(&u).unwrap();
    assert!(c.passwords.is_empty());
    assert!(c.sessions.is_none());
}

#[test]
fn stray_temp_file_does_not_break_the_vault() {
    let (_d, v) = vault();
    let u = v.create(PASSWORD).unwrap();
    // A crash between the temp write and the rename leaves a stray file behind.
    let stray = v
        .path()
        .parent()
        .unwrap()
        .join(".passwords.maine.tmp-deadbeef");
    fs::write(&stray, b"half a vault").unwrap();
    assert!(v.unlock(PASSWORD).is_ok());
    v.update(&u, |c| c.hosts.push(Host::new("A", "a.example")))
        .unwrap();
    assert_eq!(v.contents(&u).unwrap().hosts.len(), 1);
}

#[test]
fn validate_blob_and_key_opens_touch_nothing() {
    let (_d, v) = vault();
    let u = v.create(PASSWORD).unwrap();
    let blob = fs::read(v.path()).unwrap();
    assert!(VaultFile::validate_blob(&blob, PASSWORD).is_ok());
    assert_eq!(
        VaultFile::validate_blob(&blob, "wrong password!!").unwrap_err(),
        VaultError::InvalidPassword
    );
    assert!(VaultFile::key_opens(&blob, &u));

    let (_d2, other) = vault();
    let other_unlocked = other.create("a completely different pw").unwrap();
    assert!(!VaultFile::key_opens(&blob, &other_unlocked));
    assert_eq!(fs::read(v.path()).unwrap(), blob);
}

#[test]
fn contents_after_the_file_is_replaced_by_another_vault_is_locked() {
    let (_d, v) = vault();
    let u = v.create(PASSWORD).unwrap();
    let (_d2, other) = vault();
    other.create("a completely different pw").unwrap();
    fs::copy(other.path(), v.path()).unwrap();
    assert_eq!(v.contents(&u).unwrap_err(), VaultError::Locked);
}

mod local_state {
    use super::*;
    use maine_core::vault::{LocalState, SavedSession};

    fn session(host: uuid::Uuid) -> SavedSession {
        SavedSession {
            id: uuid::Uuid::new_v4(),
            host_id: host,
            name: "Session 1".into(),
            history: "$ ls\nfile".into(),
            saved_at: 804_692_800.5,
            uses_default_name: Some(true),
            number: Some(1),
        }
    }

    #[test]
    fn roundtrip_sessions_collapsed_groups_and_opaque_agent_data() {
        let (_d, v) = vault();
        let u = v.create(PASSWORD).unwrap();
        assert_eq!(v.local_state(&u).unwrap(), LocalState::default());
        let s = session(uuid::Uuid::new_v4());
        let group = uuid::Uuid::new_v4();
        v.update_local_state(&u, |l| {
            l.sessions.push(s.clone());
            l.collapsed_groups.push(group);
            l.agent_access = Some(serde_json::json!({"allowedHosts": ["x"], "futureField": 7}));
            l.agent_audit.push(serde_json::json!({"tool": "run"}));
        })
        .unwrap();
        let again = VaultFile::new(v.path()).unlock(PASSWORD).unwrap();
        let l = v.local_state(&again).unwrap();
        assert_eq!(l.sessions, vec![s]);
        assert_eq!(l.collapsed_groups, vec![group]);
        assert_eq!(l.agent_access.unwrap()["futureField"], 7);
        assert_eq!(l.agent_audit.len(), 1);
    }

    #[test]
    fn file_is_0600_sealed_and_never_contains_plaintext() {
        let (_d, v) = vault();
        let u = v.create(PASSWORD).unwrap();
        v.update_local_state(&u, |l| l.sessions.push(session(uuid::Uuid::new_v4())))
            .unwrap();
        let path = v.local_state_path(u.id);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("ls") || !text.contains("file"),
            "history must be sealed"
        );
        assert!(!text.contains("Session 1"));
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        assert_eq!(
            name,
            format!(
                "sessions-{}.maine",
                u.id.hyphenated().to_string().to_uppercase()
            )
        );
    }

    #[test]
    fn unchanged_update_does_not_write() {
        let (_d, v) = vault();
        let u = v.create(PASSWORD).unwrap();
        v.update_local_state(&u, |_| {}).unwrap();
        assert!(!v.local_state_path(u.id).exists());
    }

    #[test]
    fn foreign_or_corrupt_file_reads_empty_and_is_replaced() {
        let (_d, v) = vault();
        let u = v.create(PASSWORD).unwrap();
        fs::write(v.local_state_path(u.id), b"garbage").unwrap();
        assert_eq!(v.local_state(&u).unwrap(), LocalState::default());
        v.update_local_state(&u, |l| l.collapsed_groups.push(uuid::Uuid::new_v4()))
            .unwrap();
        assert_eq!(v.local_state(&u).unwrap().collapsed_groups.len(), 1);
    }

    #[test]
    fn locked_when_the_key_belongs_to_another_vault() {
        let (_d, v) = vault();
        v.create(PASSWORD).unwrap();
        let (_d2, other) = vault();
        let foreign = other.create("a completely different pw").unwrap();
        assert_eq!(v.local_state(&foreign).unwrap_err(), VaultError::Locked);
    }
}
