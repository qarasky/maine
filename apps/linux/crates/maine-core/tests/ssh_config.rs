use maine_core::ssh::{SshConfigEntry, SshConfigFile};
use std::fs;
use std::path::PathBuf;

fn write(dir: &tempfile::TempDir, name: &str, text: &str) -> PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, text).unwrap();
    path
}

#[test]
fn missing_file_is_not_an_error() {
    let c = SshConfigFile::load(std::path::Path::new("/nonexistent/.ssh/config"));
    assert!(c.entries.is_empty());
    assert!(c.entry("anything").is_none());
}

#[test]
fn reads_hostname_user_and_port() {
    let d = tempfile::tempdir().unwrap();
    let p = write(&d, "config", "# my hosts\nHost web\n    HostName example.com\n    User deploy\n    Port 2222\n\nHost db\n    HostName db.example.com\n");
    let c = SshConfigFile::load(&p);
    assert_eq!(c.entries.len(), 2);
    let web = c.entry("web").unwrap();
    assert_eq!(
        (web.hostname.as_deref(), web.user.as_deref(), web.port),
        (Some("example.com"), Some("deploy"), Some(2222))
    );
    assert_eq!(
        c.entry("db").unwrap().hostname.as_deref(),
        Some("db.example.com")
    );
}

#[test]
fn ignores_directives_maine_does_not_model() {
    let d = tempfile::tempdir().unwrap();
    let p = write(
        &d,
        "config",
        "Host withkeys\n    IdentityFile ~/.ssh/id_ed25519\n    ProxyJump bastion\n",
    );
    let e = SshConfigFile::load(&p).entry("withkeys").cloned().unwrap();
    assert_eq!((e.hostname, e.user, e.port), (None, None, None));
}

#[test]
fn wildcard_entries_are_not_connectable() {
    let d = tempfile::tempdir().unwrap();
    let p = write(
        &d,
        "config",
        "Host *.internal example.com\n    User me\n\nHost *\n    Compression yes\n",
    );
    let entries = SshConfigFile::load(&p).entries;
    assert_eq!(entries.len(), 3);
    let ok: Vec<_> = entries
        .iter()
        .filter(|e| SshConfigEntry::is_connectable(&e.pattern))
        .map(|e| e.pattern.as_str())
        .collect();
    assert_eq!(ok, ["example.com"]);
}

#[test]
fn directives_are_case_insensitive_and_comments_dropped() {
    let d = tempfile::tempdir().unwrap();
    let p = write(&d, "config", "# leading comment\nhost build # trailing comment\nHOSTNAME build.example.com\nUsEr builder\nport 22\n");
    let e = SshConfigFile::load(&p).entry("build").cloned().unwrap();
    assert_eq!(
        (e.hostname.as_deref(), e.user.as_deref(), e.port),
        (Some("build.example.com"), Some("builder"), Some(22))
    );
}

#[test]
fn hash_inside_a_value_survives() {
    let d = tempfile::tempdir().unwrap();
    let p = write(&d, "config", "Host h\n    HostName a#b.example.com\n");
    assert_eq!(
        SshConfigFile::load(&p)
            .entry("h")
            .unwrap()
            .hostname
            .as_deref(),
        Some("a#b.example.com")
    );
}

#[test]
fn keyword_equals_value_and_quotes() {
    let d = tempfile::tempdir().unwrap();
    let p = write(
        &d,
        "config",
        "Host eq\nHostName=eq.example.com\nPort=2022\nUser \"quoted\"\n",
    );
    let e = SshConfigFile::load(&p).entry("eq").cloned().unwrap();
    assert_eq!(
        (e.hostname.as_deref(), e.port, e.user.as_deref()),
        (Some("eq.example.com"), Some(2022), Some("quoted"))
    );
}

#[test]
fn first_value_wins_and_later_block_fills_gaps() {
    let d = tempfile::tempdir().unwrap();
    let p = write(&d, "config", "Host repeat\n    User first\n    User second\n\nHost split\n    HostName split.example.com\n\nHost split\n    User later\n");
    let c = SshConfigFile::load(&p);
    assert_eq!(c.entry("repeat").unwrap().user.as_deref(), Some("first"));
    let s = c.entry("split").unwrap();
    assert_eq!(
        (s.hostname.as_deref(), s.user.as_deref()),
        (Some("split.example.com"), Some("later"))
    );
}

#[test]
fn multi_pattern_host_line_creates_every_entry() {
    let d = tempfile::tempdir().unwrap();
    let p = write(&d, "config", "Host a b\n    User shared\n");
    let c = SshConfigFile::load(&p);
    assert_eq!(c.entry("a").unwrap().user.as_deref(), Some("shared"));
    assert_eq!(c.entry("b").unwrap().user.as_deref(), Some("shared"));
}

#[test]
fn include_brings_in_other_files() {
    let d = tempfile::tempdir().unwrap();
    write(
        &d,
        "included.conf",
        "Host extra\n    HostName extra.example.com\n",
    );
    let p = write(
        &d,
        "config",
        "Include included.conf\nHost main\n    HostName main.example.com\n",
    );
    let c = SshConfigFile::load(&p);
    assert_eq!(
        c.entry("main").unwrap().hostname.as_deref(),
        Some("main.example.com")
    );
    assert_eq!(
        c.entry("extra").unwrap().hostname.as_deref(),
        Some("extra.example.com")
    );
}

#[test]
fn include_cycle_terminates() {
    let d = tempfile::tempdir().unwrap();
    write(&d, "a.conf", "Include b.conf\nHost a\n    User ua\n");
    write(&d, "b.conf", "Include a.conf\nHost b\n    User ub\n");
    let p = write(&d, "config", "Include a.conf\n");
    let c = SshConfigFile::load(&p);
    assert_eq!(c.entry("a").unwrap().user.as_deref(), Some("ua"));
    assert_eq!(c.entry("b").unwrap().user.as_deref(), Some("ub"));
}

#[test]
fn include_glob_and_braces() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("conf.d")).unwrap();
    write(&d, "conf.d/one.conf", "Host one\n    User u1\n");
    write(&d, "conf.d/two.conf", "Host two\n    User u2\n");
    write(&d, "x.conf", "Host x\n    User ux\n");
    write(&d, "y.conf", "Host y\n    User uy\n");
    let p = write(&d, "config", "Include conf.d/*.conf\nInclude {x,y}.conf\n");
    let c = SshConfigFile::load(&p);
    for (host, user) in [("one", "u1"), ("two", "u2"), ("x", "ux"), ("y", "uy")] {
        assert_eq!(c.entry(host).unwrap().user.as_deref(), Some(user), "{host}");
    }
}

#[test]
fn entry_to_host_only_for_connectable_patterns() {
    let e = SshConfigEntry {
        pattern: "web".into(),
        hostname: None,
        user: Some("deploy".into()),
        port: Some(2222),
    };
    let h = e.to_host().unwrap();
    assert_eq!(
        (
            h.name.as_str(),
            h.hostname.as_str(),
            h.username.as_str(),
            h.port
        ),
        ("web", "web", "deploy", 2222)
    );
    assert!(SshConfigEntry {
        pattern: "*".into(),
        ..e
    }
    .to_host()
    .is_none());
}

#[test]
fn the_real_configuration_file_loads_without_crashing() {
    let _ = SshConfigFile::load(&SshConfigFile::default_path());
}
