//! Interop proof: `vault-golden-v2.json` was produced by the Swift
//! implementation (apps/android/tools/make-golden-vault.swift). It is a
//! published throwaway test vector, never a real vault.

use maine_core::vault::VaultFile;
use maine_core::VaultError;
use std::path::PathBuf;
use uuid::Uuid;

const PASSWORD: &str = "golden-horse-battery-42";

fn golden() -> (tempfile::TempDir, VaultFile) {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault-golden-v2.json");
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("passwords.maine");
    std::fs::copy(fixture, &copy).unwrap();
    (dir, VaultFile::new(copy))
}

#[test]
fn reads_macos_golden_vault() {
    let (_d, v) = golden();
    let u = v.unlock(PASSWORD).unwrap();
    let c = v.contents(&u).unwrap();

    assert_eq!(c.hosts.len(), 2);
    let web = c.hosts.iter().find(|h| h.name == "Golden web").unwrap();
    assert_eq!(web.hostname, "203.0.113.10");
    assert_eq!(web.port, 2222);
    assert_eq!(web.username, "deploy");
    assert_eq!(
        web.id,
        Uuid::parse_str("0F8FAD5B-D9CB-469F-A165-70867728950E").unwrap()
    );
    assert_eq!(
        c.passwords.get(&web.id).map(String::as_str),
        Some("golden-password-1")
    );
    assert_eq!(
        c.passwords
            .get(&Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap())
            .map(String::as_str),
        Some("golden-password-2")
    );

    let sessions = c.sessions.expect("legacy sessions are readable");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].name, "Session 1");
    assert_eq!(sessions[0].history, "golden scrollback\n$ ");
    assert!((sessions[0].saved_at - 804_692_800.5).abs() < 0.001);
}

#[test]
fn golden_roundtrip_stays_readable() {
    let (_d, v) = golden();
    let u = v.unlock(PASSWORD).unwrap();
    let before = v.contents(&u).unwrap();
    v.update(&u, |_| {}).unwrap();

    let reopened = VaultFile::new(v.path());
    let u2 = reopened.unlock(PASSWORD).unwrap();
    let after = reopened.contents(&u2).unwrap();
    assert_eq!(after.hosts, before.hosts);
    assert_eq!(after.passwords, before.passwords);
    assert!(
        after.sessions.is_none(),
        "legacy sessions are dropped on write"
    );
}

#[test]
fn golden_wrong_password_fails() {
    let (_d, v) = golden();
    assert_eq!(
        v.unlock("golden-horse-battery-43").unwrap_err(),
        VaultError::InvalidPassword
    );
}
