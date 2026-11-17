use maine_core::model::Host;
use maine_core::VaultError;
use maine_engine::askpass::Broker;
use maine_engine::{Engine, EngineState, Launcher, Paths, SessionState};
use maine_term::{SessionConfig, TermSession};
use std::sync::Arc;
use std::time::{Duration, Instant};

const PASSWORD: &str = "correct horse battery";

/// Runs a shell script instead of ssh: `sleep 30` keeps a session alive.
struct ShellLauncher(&'static str);

impl Launcher for ShellLauncher {
    fn launch(
        &self,
        _host: &Host,
        cols: u16,
        rows: u16,
        history: Option<&str>,
    ) -> Result<TermSession, String> {
        let mut c = SessionConfig::new("/bin/sh", vec!["-c".into(), self.0.into()]);
        c.cols = cols;
        c.rows = rows;
        c.history = history.map(str::to_string);
        TermSession::spawn(c)
    }
}

struct FailingLauncher;
impl Launcher for FailingLauncher {
    fn launch(&self, _: &Host, _: u16, _: u16, _: Option<&str>) -> Result<TermSession, String> {
        Err("no ssh here".into())
    }
}

fn engine_with(dir: &std::path::Path, launcher: Arc<dyn Launcher>) -> Engine {
    Engine::with_launcher(Paths::under(dir), Broker::new(), launcher)
}

fn engine(dir: &std::path::Path) -> Engine {
    engine_with(
        dir,
        Arc::new(ShellLauncher("echo hello-from-session; sleep 30")),
    )
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}");
}

fn host(name: &str) -> Host {
    Host::new(name, format!("{}.example", name.to_lowercase()))
}

#[test]
fn lifecycle_no_vault_create_lock_unlock_wrong_password() {
    let d = tempfile::tempdir().unwrap();
    let mut e = engine(d.path());
    assert_eq!(e.state(), EngineState::NoVault);
    assert_eq!(
        e.create_vault("short").unwrap_err(),
        VaultError::WeakPassword
    );
    e.create_vault(PASSWORD).unwrap();
    assert_eq!(e.state(), EngineState::Unlocked);
    e.lock();
    assert_eq!(e.state(), EngineState::Locked);
    assert_eq!(
        e.unlock("not the password").unwrap_err(),
        VaultError::InvalidPassword
    );
    assert_eq!(e.state(), EngineState::Locked);
    e.unlock(PASSWORD).unwrap();
    assert_eq!(e.state(), EngineState::Unlocked);
}

#[test]
fn hosts_are_validated_saved_with_password_and_survive_relock() {
    let d = tempfile::tempdir().unwrap();
    let mut e = engine(d.path());
    e.create_vault(PASSWORD).unwrap();
    let mut bad = host("Bad");
    bad.port = 0;
    assert!(e.save_host(bad, "").unwrap_err().0.contains("port"));

    let h = host("Web");
    e.save_host(h.clone(), "s3cret").unwrap();
    assert_eq!(e.hosts(), std::slice::from_ref(&h));
    assert_eq!(e.saved_password(h.id).as_deref(), Some("s3cret"));
    assert_eq!(
        e.credential_text(h.id).unwrap(),
        "IP: web.example\npassword: s3cret"
    );

    // An empty password means key login: nothing stored.
    e.save_host(h.clone(), "").unwrap();
    assert_eq!(e.saved_password(h.id), None);

    e.lock();
    assert_eq!(e.hosts().len(), 0, "locked: nothing readable");
    assert_eq!(e.saved_password(h.id), None);
    assert!(e.save_host(host("X"), "").is_err());
    e.unlock(PASSWORD).unwrap();
    assert_eq!(e.hosts(), [h]);
}

#[test]
fn layout_changes_are_atomic_and_collapse_state_is_device_local() {
    let d = tempfile::tempdir().unwrap();
    let mut e = engine(d.path());
    e.create_vault(PASSWORD).unwrap();
    let (a, b) = (host("A"), host("B"));
    e.save_host(a.clone(), "").unwrap();
    e.save_host(b.clone(), "").unwrap();
    let mut group = None;
    e.update_layout(|l| {
        group = Some(l.add_group("Prod").id);
        l.move_host(a.id, group, None);
        Ok(())
    })
    .unwrap();
    let group = group.unwrap();
    assert_eq!(e.layout().hosts_in(group).len(), 1);

    // A failing change leaves the layout untouched.
    let before = e.layout().clone();
    assert!(e.update_layout(|l| l.rename_group(group, "  ")).is_err());
    assert_eq!(e.layout(), &before);

    e.set_collapsed(group, true).unwrap();
    assert!(e.is_collapsed(group));
    e.lock();
    e.unlock(PASSWORD).unwrap();
    assert!(e.is_collapsed(group));
    assert_eq!(e.groups().len(), 1);

    // Deleting the group forgets its collapsed flag.
    e.update_layout(|l| {
        l.delete_group(group);
        Ok(())
    })
    .unwrap();
    e.reload().unwrap();
    assert!(!e.is_collapsed(group));
}

#[test]
fn opening_a_host_reveals_the_existing_session_and_new_session_adds_another() {
    let d = tempfile::tempdir().unwrap();
    let mut e = engine(d.path());
    e.create_vault(PASSWORD).unwrap();
    let h = host("Web");
    e.save_host(h.clone(), "").unwrap();

    let first = e.sessions_mut().open(&h);
    assert_eq!(
        e.sessions_mut().open(&h),
        first,
        "same host reveals the same session"
    );
    let second = e.sessions_mut().new_session(&h);
    assert_ne!(first, second);
    let labels: Vec<String> = e
        .sessions()
        .list()
        .iter()
        .map(|s| s.label.clone())
        .collect();
    assert_eq!(labels, ["Session 1", "Session 2"]);
    assert_eq!(e.sessions().selected(), Some(second));

    let term = e.sessions().terminal(first).unwrap();
    wait_until("output", || {
        term.snapshot().text().contains("hello-from-session")
    });
    wait_until("connected", || {
        e.sessions().state(first) == Some(SessionState::Connected)
    });
    e.quit();
}

#[test]
fn close_deletes_the_record_but_quit_keeps_it_and_restore_brings_it_back_with_history() {
    let d = tempfile::tempdir().unwrap();
    let h = host("Web");
    {
        let mut e = engine(d.path());
        e.create_vault(PASSWORD).unwrap();
        e.save_host(h.clone(), "").unwrap();
        let keep = e.sessions_mut().open(&h);
        let drop_me = e.sessions_mut().new_session(&h);
        e.sessions_mut().rename(keep, "Deploy box").unwrap();
        let term = e.sessions().terminal(keep).unwrap();
        wait_until("output", || {
            term.snapshot().text().contains("hello-from-session")
        });
        e.tick();
        let file = e.vault_file().clone();
        let key = e.unlocked().unwrap().clone();
        assert_eq!(file.local_state(&key).unwrap().sessions.len(), 2);

        // Explicit close deletes its record first.
        e.sessions_mut().close_session(drop_me).unwrap();
        assert_eq!(file.local_state(&key).unwrap().sessions.len(), 1);
        // Quit stops processes and keeps the remaining record.
        e.quit();
        assert_eq!(file.local_state(&key).unwrap().sessions.len(), 1);
    }
    // A fresh launch: unlock restores the session with its name and scrollback.
    let mut e = engine(d.path());
    e.unlock(PASSWORD).unwrap();
    let list = e.sessions().list();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].label, "Deploy box");
    assert!(list[0].saved);
    let term = e.sessions().terminal(list[0].id).unwrap();
    wait_until("restored history shown", || {
        term.snapshot().text().contains("hello-from-session")
    });
    e.quit();
}

#[test]
fn lock_keeps_sessions_running_and_unlock_does_not_duplicate_them() {
    let d = tempfile::tempdir().unwrap();
    let mut e = engine(d.path());
    e.create_vault(PASSWORD).unwrap();
    let h = host("Web");
    e.save_host(h.clone(), "").unwrap();
    let id = e.sessions_mut().open(&h);
    e.lock();
    assert!(e.sessions().state(id).unwrap().is_live());
    e.unlock(PASSWORD).unwrap();
    assert_eq!(
        e.sessions().list().len(),
        1,
        "restore skips sessions that are already open"
    );
    assert_eq!(e.sessions().list()[0].id, id);
    e.quit();
}

#[test]
fn explicit_disconnect_is_not_an_error_and_reopening_replaces_in_place() {
    let d = tempfile::tempdir().unwrap();
    let mut e = engine(d.path());
    e.create_vault(PASSWORD).unwrap();
    let h = host("Web");
    e.save_host(h.clone(), "").unwrap();
    let id = e.sessions_mut().open(&h);
    e.sessions_mut().disconnect(id);
    wait_until("disconnected", || {
        matches!(e.sessions().state(id), Some(SessionState::Disconnected(_)))
    });
    assert_eq!(e.sessions().state(id).unwrap().notice(), None);

    // Opening the host again reconnects: same slot, new session id, selected.
    let again = e.sessions_mut().open(&h);
    assert_ne!(again, id);
    assert!(!e.sessions().is_open(id));
    assert_eq!(e.sessions().list().len(), 1);
    assert_eq!(e.sessions().selected(), Some(again));
    assert_eq!(e.sessions().list()[0].label, "Session 1");
    e.quit();
}

#[test]
fn a_launch_failure_becomes_a_failed_state_with_the_message() {
    let d = tempfile::tempdir().unwrap();
    let mut e = engine_with(d.path(), Arc::new(FailingLauncher));
    e.create_vault(PASSWORD).unwrap();
    let h = host("Web");
    e.save_host(h.clone(), "").unwrap();
    let id = e.sessions_mut().open(&h);
    assert_eq!(
        e.sessions().state(id),
        Some(SessionState::Failed("no ssh here".into()))
    );
    assert_eq!(
        e.sessions().state(id).unwrap().notice().as_deref(),
        Some("no ssh here")
    );
    assert!(e.sessions().terminal(id).is_none());
}

#[test]
fn non_zero_exit_gets_a_notice() {
    let d = tempfile::tempdir().unwrap();
    let mut e = engine_with(d.path(), Arc::new(ShellLauncher("exit 255")));
    e.create_vault(PASSWORD).unwrap();
    let h = host("Web");
    e.save_host(h.clone(), "").unwrap();
    let id = e.sessions_mut().open(&h);
    wait_until("closed", || {
        matches!(e.sessions().state(id), Some(SessionState::Closed(_)))
    });
    assert_eq!(
        e.sessions().state(id),
        Some(SessionState::Closed(Some(255)))
    );
    assert!(e
        .sessions()
        .state(id)
        .unwrap()
        .notice()
        .unwrap()
        .contains("255"));
}

#[test]
fn deleting_a_host_closes_its_sessions_and_drops_records_and_password() {
    let d = tempfile::tempdir().unwrap();
    let mut e = engine(d.path());
    e.create_vault(PASSWORD).unwrap();
    let (a, b) = (host("A"), host("B"));
    e.save_host(a.clone(), "pw").unwrap();
    e.save_host(b.clone(), "").unwrap();
    e.sessions_mut().open(&a);
    let keep = e.sessions_mut().open(&b);
    e.tick();
    e.delete_host(a.id).unwrap();
    assert_eq!(e.hosts(), std::slice::from_ref(&b));
    assert_eq!(e.saved_password(a.id), None);
    let list = e.sessions().list();
    assert_eq!(list.len(), 1);
    assert_eq!(
        list[0].id, keep,
        "the surviving host keeps its running terminal"
    );
    let local = e.vault_file().local_state(e.unlocked().unwrap()).unwrap();
    assert!(local.sessions.iter().all(|s| s.host_id == b.id));
    e.quit();
}

#[test]
fn adopting_a_remote_change_closes_only_sessions_whose_hosts_are_gone() {
    let d = tempfile::tempdir().unwrap();
    let mut e = engine(d.path());
    e.create_vault(PASSWORD).unwrap();
    let (a, b) = (host("A"), host("B"));
    e.save_host(a.clone(), "").unwrap();
    e.save_host(b.clone(), "").unwrap();
    let sa = e.sessions_mut().open(&a);
    let sb = e.sessions_mut().open(&b);
    let term_b = e.sessions().terminal(sb).unwrap();
    // Another device removed host A: simulate by editing the vault file directly.
    let file = e.vault_file().clone();
    let key = e.unlocked().unwrap().clone();
    file.update(&key, |c| c.hosts.retain(|h| h.id != a.id))
        .unwrap();
    e.adopt_remote_change().unwrap();
    assert!(!e.sessions().is_open(sa));
    assert!(e.sessions().is_open(sb));
    assert!(
        term_b.is_running(),
        "a surviving terminal is never restarted"
    );
    e.quit();
}

#[test]
fn rename_requires_a_name_and_a_saved_session_updates_its_record() {
    let d = tempfile::tempdir().unwrap();
    let mut e = engine(d.path());
    e.create_vault(PASSWORD).unwrap();
    let h = host("Web");
    e.save_host(h.clone(), "").unwrap();
    let id = e.sessions_mut().open(&h);
    assert_eq!(
        e.sessions_mut().rename(id, "  ").unwrap_err(),
        "Enter a session name."
    );
    e.tick();
    e.sessions_mut().rename(id, "Prod shell").unwrap();
    let local = e.vault_file().local_state(e.unlocked().unwrap()).unwrap();
    assert_eq!(local.sessions[0].name, "Prod shell");
    assert_eq!(local.sessions[0].uses_default_name, Some(false));
    e.quit();
}

#[test]
fn ssh_config_candidates_skip_wildcards_and_hosts_already_saved() {
    let d = tempfile::tempdir().unwrap();
    let mut e = engine(d.path());
    e.create_vault(PASSWORD).unwrap();
    e.save_host(Host::new("Known", "known"), "").unwrap();
    let cfg = d.path().join("ssh_config");
    std::fs::write(&cfg, "Host known\n  User a\nHost fresh\n  User b\n  Port 2222\nHost *.internal\n  User c\nHost *\n  Compression yes\n").unwrap();
    let candidates = e.ssh_config_candidates(&maine_core::ssh::SshConfigFile::load(&cfg));
    assert_eq!(candidates.len(), 1);
    assert_eq!(
        (
            candidates[0].hostname.as_str(),
            candidates[0].port,
            candidates[0].username.as_str()
        ),
        ("fresh", 2222, "b")
    );
}

#[test]
fn sidebar_json_describes_state_groups_hosts_and_sessions() {
    let d = tempfile::tempdir().unwrap();
    let mut e = engine(d.path());
    assert_eq!(maine_engine::view::sidebar(&e)["state"], 0);
    e.create_vault(PASSWORD).unwrap();
    let (a, b) = (host("A"), host("B"));
    e.save_host(a.clone(), "pw").unwrap();
    e.save_host(b.clone(), "").unwrap();
    let mut group = None;
    e.update_layout(|l| {
        group = Some(l.add_group("Prod").id);
        l.move_host(b.id, group, None);
        Ok(())
    })
    .unwrap();
    let id = e.sessions_mut().open(&a);
    let v = maine_engine::view::sidebar(&e);
    assert_eq!(v["state"], 2);
    assert_eq!(v["hostCount"], 2);
    assert_eq!(v["selected"], id.to_string());
    assert_eq!(v["ungrouped"][0]["name"], "A");
    assert_eq!(v["ungrouped"][0]["hasPassword"], true);
    assert_eq!(v["ungrouped"][0]["sessions"][0]["label"], "Session 1");
    assert_eq!(v["groups"][0]["name"], "Prod");
    assert_eq!(v["groups"][0]["hosts"][0]["name"], "B");
    assert_eq!(v["groups"][0]["hosts"][0]["hasPassword"], false);
    assert_eq!(v["groups"][0]["collapsed"], false);
    assert!(
        !v.to_string().contains("pw\""),
        "passwords never appear in the sidebar JSON"
    );
    let editor = maine_engine::view::editor_host(&e, a.id);
    assert_eq!(editor["password"], "pw");
    e.quit();
}

mod agent_sessions {
    use super::*;
    use maine_engine::AgentLink;
    use uuid::Uuid;

    fn link(handed_off: bool) -> AgentLink {
        AgentLink {
            client_id: Uuid::new_v4(),
            client_name: "Claude Code".into(),
            handed_off,
        }
    }

    #[test]
    fn an_agent_session_is_not_selected_not_saved_not_restored_and_not_reached_by_host_clicks() {
        let d = tempfile::tempdir().unwrap();
        let h = host("Web");
        {
            let mut e = engine(d.path());
            e.create_vault(PASSWORD).unwrap();
            e.save_host(h.clone(), "").unwrap();
            let mine = e.sessions_mut().open(&h);
            let agent = e
                .sessions_mut()
                .open_agent_session(&h, link(false), Some("  build  "));
            assert_eq!(
                e.sessions().selected(),
                Some(mine),
                "the user's focus stays where it is"
            );
            let info = e
                .sessions()
                .list()
                .into_iter()
                .find(|s| s.id == agent)
                .unwrap();
            assert_eq!(info.label, "build");
            assert!(info.ephemeral && info.agent.is_some());
            // Host-level actions ignore agent sessions.
            assert_eq!(e.sessions_mut().open(&h), mine);
            e.tick();
            let local = e.vault_file().local_state(e.unlocked().unwrap()).unwrap();
            assert_eq!(
                local.sessions.len(),
                1,
                "only the user's session has a record"
            );
            e.quit();
        }
        let mut e = engine(d.path());
        e.unlock(PASSWORD).unwrap();
        assert_eq!(
            e.sessions().list().len(),
            1,
            "agent sessions are never restored"
        );
        e.quit();
    }

    #[test]
    fn ending_control_marks_an_agent_session_until_kept_and_keeping_makes_it_an_ordinary_session() {
        let d = tempfile::tempdir().unwrap();
        let mut e = engine(d.path());
        e.create_vault(PASSWORD).unwrap();
        let h = host("Web");
        e.save_host(h.clone(), "").unwrap();
        let agent = e.sessions_mut().open_agent_session(&h, link(false), None);
        e.sessions_mut().end_agent_control(agent);
        let info = e
            .sessions()
            .list()
            .into_iter()
            .find(|s| s.id == agent)
            .unwrap();
        assert_eq!(info.former_agent.as_deref(), Some("Claude Code"));
        assert!(info.ephemeral && info.agent.is_none());
        e.tick();
        assert!(e
            .vault_file()
            .local_state(e.unlocked().unwrap())
            .unwrap()
            .sessions
            .is_empty());

        e.sessions_mut().keep_session(agent);
        let info = e
            .sessions()
            .list()
            .into_iter()
            .find(|s| s.id == agent)
            .unwrap();
        assert!(!info.ephemeral && info.former_agent.is_none());
        e.tick();
        assert_eq!(
            e.vault_file()
                .local_state(e.unlocked().unwrap())
                .unwrap()
                .sessions
                .len(),
            1
        );
        e.quit();
    }

    #[test]
    fn a_handed_off_session_stays_saved_and_returns_to_the_user_without_a_mark() {
        let d = tempfile::tempdir().unwrap();
        let mut e = engine(d.path());
        e.create_vault(PASSWORD).unwrap();
        let h = host("Web");
        e.save_host(h.clone(), "").unwrap();
        let mine = e.sessions_mut().open(&h);
        e.sessions_mut().attach_agent(mine, link(true));
        let info = e
            .sessions()
            .list()
            .into_iter()
            .find(|s| s.id == mine)
            .unwrap();
        assert!(
            info.agent.is_some() && !info.ephemeral,
            "handed-off sessions remain ordinary saved sessions"
        );
        // An agent session that gets another handoff stays ephemeral (it was never the user's).
        e.sessions_mut().end_agent_control(mine);
        let info = e
            .sessions()
            .list()
            .into_iter()
            .find(|s| s.id == mine)
            .unwrap();
        assert!(info.agent.is_none() && info.former_agent.is_none() && !info.ephemeral);
        // An agent-driven session cannot be reconnected underneath its driver.
        e.sessions_mut().attach_agent(mine, link(true));
        assert_eq!(e.sessions_mut().reconnect_session(mine, None), None);
        e.quit();
    }
}
