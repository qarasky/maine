//! The driver against real PTYs (`/bin/sh` and bash stand in for the remote
//! shell, no ssh) and against scripted sessions with a manual clock. Ports of
//! the Swift `AgentDriverTests`; the ones about `SessionManager` persistence
//! belong to the app and have no counterpart here.

mod common;

use common::*;
use maine_agents::{
    AgentDriver, AgentKey, AgentSession, Clock, DriverError, ManualClock, MASK_REPLACEMENT,
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

const T10: Duration = Duration::from_secs(10);

// MARK: - Real PTY

#[test]
fn run_returns_output_and_exit_code() {
    let d = driven_shell();
    let result = d.driver.run("echo hi; false", T10, None).unwrap();
    assert!(result.completed);
    assert_eq!(result.exit_code, Some(1));
    assert_eq!(result.mode, "marker");
    assert_eq!(result.output.trim(), "hi");
    assert!(!d.driver.is_busy());

    let multiline = d
        .driver
        .run("echo one # a comment\necho two", T10, None)
        .unwrap();
    assert_eq!(multiline.exit_code, Some(0));
    assert_eq!(multiline.output.trim(), "one\ntwo");
}

#[test]
fn timeouts_return_partial_output_and_leave_the_command_running() {
    let d = driven_shell();
    let result = d
        .driver
        .run(
            "echo start; sleep 3; echo end",
            Duration::from_secs(1),
            None,
        )
        .unwrap();
    assert!(!result.completed);
    assert_eq!(result.exit_code, None);
    assert!(result.output.contains("start"));
    assert!(d.driver.is_busy());
    let mut read = d
        .driver
        .read_output(Some(result.cursor), Some(Duration::from_secs(6)), 65_536);
    let deadline = Instant::now() + Duration::from_secs(6);
    while read.busy && Instant::now() < deadline {
        read = d
            .driver
            .read_output(Some(result.cursor), Some(Duration::from_secs(1)), 65_536);
    }
    assert!(d
        .driver
        .read_output(Some(result.cursor), None, 65_536)
        .output
        .contains("end"));
    assert!(!d.driver.is_busy());
}

/// A progress bar redrawn behind `\r` is new output: a waiting read wakes for
/// it and returns the redrawn line, not an empty result.
#[test]
fn a_waiting_read_wakes_for_an_in_place_redraw() {
    let d = driven_shell();
    let result = d
        .driver
        .run(
            "printf '\\r[##  ] 20%%'; sleep 1; printf '\\r[####] 90%%'; sleep 4",
            Duration::from_millis(500),
            None,
        )
        .unwrap();
    assert!(!result.completed);
    let started = Instant::now();
    let read = d.driver.read_output(Some(result.cursor), Some(T10), 65_536);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert!(read.output.contains("[####] 90%"), "{:?}", read.output);
    assert!(read.cursor > result.cursor);
    assert!(read.busy);
}

#[test]
fn run_is_refused_until_the_shell_is_up() {
    // ssh is still "connecting": no output yet, then a login that takes 2 s.
    let session = spawn_script("sleep 2; exec /bin/sh");
    let driver = drive(&session, system_clock());
    let _end = Guard(session.clone());
    assert_eq!(
        driver.run("echo early", Duration::from_secs(5), None),
        Err(DriverError::NotReady)
    );
    assert!(!driver.is_busy());
    driver.settle_after_login(Duration::from_secs(15));
    let result = driver.run("echo late", T10, None).unwrap();
    assert!(result.output.contains("late"));
}

#[test]
fn a_password_prompt_is_not_a_ready_shell() {
    let session = spawn_script("printf 'ubuntu@host password: '; sleep 30");
    let driver = drive(&session, system_clock());
    let _end = Guard(session.clone());
    let started = Instant::now();
    driver.settle_after_login(Duration::from_secs(20));
    // The agent gets control back to answer the prompt, not after 20 s.
    assert!(started.elapsed() < T10, "{:?}", started.elapsed());
    assert!(!driver.is_ready());
    assert_eq!(
        driver.run("echo no", Duration::from_secs(5), None),
        Err(DriverError::NotReady)
    );
}

#[test]
fn a_busy_session_refuses_another_run() {
    let d = driven_shell();
    d.driver
        .run("sleep 2", Duration::from_millis(200), None)
        .unwrap();
    assert_eq!(
        d.driver.run("echo no", Duration::from_secs(1), None),
        Err(DriverError::Busy)
    );
}

#[test]
fn paused_drivers_refuse_input() {
    let d = driven_shell();
    d.driver.set_paused(true);
    assert_eq!(
        d.driver.run("echo no", Duration::from_secs(1), None),
        Err(DriverError::Paused)
    );
    assert_eq!(
        d.driver.send_keys(&[AgentKey::Named("enter".into())]),
        Err(DriverError::Paused)
    );
    d.driver.set_paused(false);
    assert!(d
        .driver
        .run("echo yes", T10, None)
        .unwrap()
        .output
        .contains("yes"));
}

#[test]
fn a_dead_session_fails_fast() {
    let d = driven_shell();
    d.driver
        .send_keys(&[
            AgentKey::Text("exit".into()),
            AgentKey::Named("enter".into()),
        ])
        .unwrap();
    assert!(wait_until(Duration::from_secs(5), || !d
        .session
        .is_running()));
    let started = Instant::now();
    assert_eq!(
        d.driver.run("echo no", T10, None),
        Err(DriverError::NotConnected)
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn ctrl_c_interrupts_a_running_command() {
    let d = driven_shell();
    d.driver
        .run("sleep 30", Duration::from_secs(1), None)
        .unwrap();
    assert!(d.driver.is_busy());
    d.driver
        .send_keys(&[AgentKey::Named("ctrl-c".into())])
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while d.driver.is_busy() && Instant::now() < deadline {
        d.driver
            .read_output(None, Some(Duration::from_millis(200)), 1);
    }
    assert!(!d.driver.is_busy());
    let after = d.driver.run("echo alive", T10, None).unwrap();
    assert!(after.output.contains("alive"));
}

#[test]
fn keys_map_to_bytes() {
    assert_eq!(
        AgentKey::parse("ctrl-c"),
        Some(AgentKey::Named("ctrl-c".into()))
    );
    assert_eq!(AgentKey::parse("ctrl-c").unwrap().bytes(), [0x03]);
    assert_eq!(AgentKey::parse("ctrl-z").unwrap().bytes(), [0x1A]);
    assert_eq!(AgentKey::parse("enter").unwrap().bytes(), [0x0D]);
    assert_eq!(AgentKey::parse("tab").unwrap().bytes(), [0x09]);
    assert_eq!(AgentKey::parse("esc").unwrap().bytes(), [0x1B]);
    assert_eq!(AgentKey::parse("backspace").unwrap().bytes(), [0x7F]);
    assert_eq!(AgentKey::parse("up").unwrap().bytes(), b"\x1b[A");
    assert_eq!(AgentKey::parse("down").unwrap().bytes(), b"\x1b[B");
    assert_eq!(AgentKey::parse("right").unwrap().bytes(), b"\x1b[C");
    assert_eq!(AgentKey::parse("left").unwrap().bytes(), b"\x1b[D");
    assert_eq!(AgentKey::parse("text:ls -la").unwrap().bytes(), b"ls -la");
    assert_eq!(AgentKey::parse("bogus"), None);
    assert_eq!(AgentKey::parse("ctrl-1"), None);
    assert_eq!(AgentKey::parse("ctrl-"), None);
    assert_eq!(AgentKey::parse("ctrl-ab"), None);
    assert_eq!(AgentKey::parse("ctrl-A"), None);
    assert!(AgentKey::parse("ctrl-c").unwrap().interrupts());
    assert!(AgentKey::parse("ctrl-d").unwrap().interrupts());
    assert!(!AgentKey::parse("ctrl-a").unwrap().interrupts());
}

#[test]
fn screen_reflects_the_visible_terminal() {
    let d = driven_shell();
    d.driver.run("printf 'row-a\\n'", T10, None).unwrap();
    let screen = d.driver.screen();
    assert!(
        screen.rows.iter().any(|row| row.contains("row-a")),
        "{:?}",
        screen.rows
    );
    assert!(!screen.alternate_screen);
}

/// Multi-line progress redrawn with cursor-up only reads right on the screen;
/// a waiting `read_screen` wakes when the screen differs from what it last
/// returned, and otherwise waits out the time without a change.
#[test]
fn a_waiting_screen_read_wakes_when_the_screen_changes() {
    // bash, like macOS' /bin/sh: its line editor prints PS2 inside the echo.
    // dash prints it in front of the output, which would put `> ` on the rows.
    let d = driven_bash();
    std::thread::sleep(Duration::from_millis(300)); // let the prompt after the probe settle
    assert!(d.driver.read_screen(None).changed); // a first read is always news
    let started = Instant::now();
    let idle = d.driver.read_screen(Some(Duration::from_secs(1)));
    assert!(!idle.changed);
    assert!(started.elapsed() >= Duration::from_secs(1));
    assert!(!idle.busy);

    d.driver
        .run(
            "printf 'a 10%%\\nb 10%%\\n'; sleep 1; printf '\\033[2Aa 90%%\\nb 90%%\\n'; sleep 4",
            Duration::from_millis(500),
            None,
        )
        .unwrap();
    let before = d.driver.read_screen(None);
    assert!(
        before.rows.iter().any(|row| row.trim() == "a 10%"),
        "{:?}",
        before.rows
    );
    let started = Instant::now();
    let next = d.driver.read_screen(Some(T10));
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert!(next.changed);
    assert!(next.busy);
    let rows: Vec<&str> = next.rows.iter().map(|row| row.trim()).collect();
    assert!(
        rows.contains(&"a 90%") && rows.contains(&"b 90%"),
        "{rows:?}"
    );
    assert!(!rows.contains(&"a 10%"), "{rows:?}"); // redrawn in place, not appended
}

#[test]
fn a_driven_session_is_flagged_so_clipboard_and_links_stay_off() {
    // The Swift test reads the pasteboard through the terminal view; here the
    // session carries the flag the UI checks before honoring OSC 52, links
    // and the bell.
    let session = spawn_shell();
    let _end = Guard(session.clone());
    assert!(!session.is_agent_driven());
    let driver = drive(&session, system_clock());
    assert!(session.is_agent_driven());
    driver.detach();
    assert!(!session.is_agent_driven());
}

#[test]
fn parallel_runs_cannot_both_pass_the_busy_check() {
    let d = driven_shell();
    d.session.note_user_input(); // the user just typed: both runs wait first
    let results: Vec<_> = std::thread::scope(|scope| {
        let first = scope.spawn(|| d.driver.run("echo A", T10, None));
        let second = scope.spawn(|| d.driver.run("echo B", T10, None));
        vec![first.join().unwrap(), second.join().unwrap()]
    });
    let busy = results
        .iter()
        .filter(|r| matches!(r, Err(DriverError::Busy)))
        .count();
    assert_eq!(busy, 1, "{results:?}");
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
}

#[test]
fn stopping_an_agent_ends_its_running_call() {
    let d = driven_shell();
    let stopped_at = std::thread::scope(|scope| {
        let run = scope.spawn(|| d.driver.run("sleep 5", Duration::from_secs(30), None));
        std::thread::sleep(Duration::from_millis(300));
        let stopped = Instant::now();
        d.driver.detach();
        let result = run.join().unwrap();
        assert_eq!(result, Err(DriverError::Stopped));
        stopped.elapsed()
    });
    assert!(stopped_at < Duration::from_secs(1), "{stopped_at:?}");
}

#[test]
fn the_stop_callback_ends_a_run_too() {
    let d = driven_shell();
    let started = Instant::now();
    let stop_after = Instant::now() + Duration::from_millis(300);
    let result = d.driver.run(
        "sleep 5",
        Duration::from_secs(30),
        Some(&|| Instant::now() > stop_after),
    );
    assert_eq!(result, Err(DriverError::Stopped));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn a_session_is_ready_once_output_arrives() {
    let d = driven_shell();
    assert!(d.driver.is_ready());
    assert!(d.session.is_running());
}

// MARK: - type_password

#[test]
fn type_password_answers_a_prompt_and_masks_it_everywhere() {
    let d = driven_shell();
    // The PTY echoes what is typed, so the secret really reaches the output.
    let result = d
        .driver
        .run(
            "printf '[sudo] password for alice: '; read -r p; echo \"got:$p\"",
            Duration::from_millis(800),
            None,
        )
        .unwrap();
    assert!(!result.completed);
    d.driver.type_password("hunter2-secret").unwrap();
    let wanted = format!("got:{MASK_REPLACEMENT}");
    let mut text = String::new();
    let deadline = Instant::now() + Duration::from_secs(8);
    while !text.contains(&wanted) && Instant::now() < deadline {
        text = d
            .driver
            .read_output(Some(0), Some(Duration::from_millis(200)), 65_536)
            .output;
    }
    assert!(text.contains(&wanted), "{text:?}");
    assert!(!text.contains("hunter2-secret"));
    assert!(text.contains(MASK_REPLACEMENT));
    assert!(!d.driver.screen().rows.join("\n").contains("hunter2-secret"));
}

#[test]
fn type_password_refuses_when_no_prompt_is_showing() {
    let d = driven_shell();
    d.driver.run("echo plain", T10, None).unwrap();
    assert_eq!(
        d.driver.type_password("never-typed"),
        Err(DriverError::NoPasswordPrompt)
    );
    let after = d.driver.run("echo ok", T10, None).unwrap();
    assert!(after.output.contains("ok"));
    assert!(!after.output.contains("never-typed"));
}

#[test]
fn type_password_refuses_passphrase_and_one_time_prompts() {
    let d = driven_shell();
    d.driver
        .run(
            "printf 'Enter passphrase for key: '; read -r p",
            Duration::from_millis(800),
            None,
        )
        .unwrap();
    assert_eq!(
        d.driver.type_password("nope"),
        Err(DriverError::NoPasswordPrompt)
    );
    d.driver
        .send_keys(&[AgentKey::Named("enter".into())])
        .unwrap();
}

// MARK: - OSC 133

/// A shell that already emits shell-integration marks from its first prompt,
/// as a remote with iTerm2/VS Code/Ghostty integration does. The first prompt
/// waits a moment so the driver's tap is surely installed.
fn integrated_shell() -> Driven {
    let mut config = shell_config("/usr/bin/bash", &["--norc", "--noprofile", "-i"]);
    config.env.push((
        "PROMPT_COMMAND".into(),
        "s=$?; [ -n \"$started\" ] || { sleep 0.3; started=1; }; printf '\\033]133;D;%d\\007\\033]133;A\\007' $s".into(),
    ));
    let term = maine_term::TermSession::spawn(config).expect("spawn bash");
    let session = Arc::new(maine_agents::TermAgentSession::new(term));
    let driver = drive(&session, system_clock());
    assert!(
        wait_until(T10, || driver.has_shell_integration()),
        "no OSC 133;A seen"
    );
    driver.settle_after_login(T10);
    Driven { session, driver }
}

#[test]
fn existing_shell_integration_replaces_the_wrapper() {
    let d = integrated_shell();
    let result = d.driver.run("echo hi; false", T10, None).unwrap();
    assert_eq!(result.mode, "osc133");
    assert!(result.completed);
    assert_eq!(result.exit_code, Some(1));
    assert_eq!(result.output.trim(), "hi");
    let again = d.driver.run("echo two", T10, None).unwrap();
    assert_eq!(again.exit_code, Some(0));
    assert_eq!(again.output.trim(), "two");
    // The human sees no wrapper noise.
    let seen = d.driver.read_output(Some(0), None, 65_536).output;
    assert!(!seen.contains("7770"), "{seen:?}");
}

#[test]
fn plain_shells_still_use_the_marker() {
    let d = driven_shell();
    assert_eq!(d.driver.run("true", T10, None).unwrap().mode, "marker");
}

#[test]
fn bash_works_with_the_marker_too() {
    let session = spawn_bash();
    let driver = drive(&session, system_clock());
    let _d = Driven {
        session: session.clone(),
        driver: driver.clone(),
    };
    driver.settle_after_login(T10);
    let result = driver.run("echo hi; (exit 4)", T10, None).unwrap();
    assert_eq!(
        (result.mode, result.exit_code, result.output.trim()),
        ("marker", Some(4), "hi")
    );
}

// MARK: - Scripted sessions on a manual clock

struct Scripted {
    session: Arc<FakeSession>,
    driver: Arc<AgentDriver>,
    clock: Arc<ManualClock>,
}

fn scripted(integration: Integration, commands: &[(&str, &str, i32)]) -> Scripted {
    let clock = Arc::new(ManualClock::new());
    let session = FakeSession::new();
    scripted_shell(&session, integration, commands);
    let driver = AgentDriver::new(
        session.clone() as Arc<dyn AgentSession>,
        Uuid::new_v4(),
        "Fake agent",
        clock.clone(),
    );
    if integration == Integration::Osc133 {
        session.emit_str(&format!(
            "\x1b]133;D;0\x07\x1b]133;A\x07{PROMPT}\x1b]133;B\x07"
        ));
    } else {
        session.emit_str(PROMPT);
    }
    Scripted {
        session,
        driver,
        clock,
    }
}

#[test]
fn a_marker_shell_completes_a_run_without_waiting() {
    let s = scripted(Integration::Marker, &[("echo hi; false", "hi\n", 1)]);
    s.driver.settle_after_login(Duration::from_secs(30));
    let started = s.clock.now();
    let result = s
        .driver
        .run("echo hi; false", Duration::from_secs(60), None)
        .unwrap();
    assert_eq!(
        (result.completed, result.exit_code, result.mode),
        (true, Some(1), "marker")
    );
    assert_eq!(result.output.trim(), "hi");
    assert!(
        s.clock.now() - started < Duration::from_secs(1),
        "no waiting for the timeout"
    );
    assert!(!s.driver.is_busy());
    // Maine's wrapper, not the bare command, went to the shell.
    assert!(s
        .session
        .sent_text()
        .contains("{ echo hi; false\n}; printf '\\033]7770;"));
}

#[test]
fn ps2_prompts_printed_after_the_echo_do_not_reach_the_output() {
    // dash and ash echo the typed wrapper first and print `> ` afterwards, in
    // front of the output. A bash-style shell prints it inside the echo.
    for ps2 in [Ps2::AfterEcho, Ps2::Inline, Ps2::MidEcho] {
        let clock = Arc::new(ManualClock::new());
        let session = FakeSession::new();
        scripted_shell_with(
            &session,
            Integration::Marker,
            ps2,
            &[
                ("echo hi", "hi\n", 0),
                ("echo '> x'", "> x\n", 0),
                ("echo hi > /dev/null; echo hi", "hi\n", 0),
            ],
        );
        let driver = AgentDriver::new(
            session.clone() as Arc<dyn AgentSession>,
            Uuid::new_v4(),
            "A",
            clock,
        );
        session.emit_str(PROMPT);
        driver.settle_after_login(Duration::from_secs(30));
        let plain = driver
            .run("echo hi", Duration::from_secs(60), None)
            .unwrap();
        assert_eq!(plain.output, "hi\n", "{ps2:?}");
        // Output that really starts with `> ` keeps it.
        let quoted = driver
            .run("echo '> x'", Duration::from_secs(60), None)
            .unwrap();
        assert_eq!(quoted.output, "> x\n", "{ps2:?}");
        // A redirection in the command is not a prompt either.
        let redirected = driver
            .run(
                "echo hi > /dev/null; echo hi",
                Duration::from_secs(60),
                None,
            )
            .unwrap();
        assert_eq!(redirected.output, "hi\n", "{ps2:?}");
    }
}

#[test]
fn a_shell_without_the_marker_falls_back_to_quiet_mode() {
    let s = scripted(Integration::Plain, &[("echo hi", "hi\n", 0)]);
    s.driver.settle_after_login(Duration::from_secs(30)); // probes for 2 s of virtual time
    let result = s
        .driver
        .run("echo hi", Duration::from_secs(60), None)
        .unwrap();
    assert_eq!(result.mode, "quiet");
    assert!(result.completed);
    assert_eq!(
        result.exit_code, None,
        "quiet mode cannot know the exit code"
    );
    assert_eq!(result.output, "hi");
    // The command was typed as is, with no wrapper.
    assert!(s.session.sent_text().ends_with("echo hi\n"));
}

#[test]
fn a_shell_with_osc133_needs_no_wrapper_and_no_probe() {
    let s = scripted(Integration::Osc133, &[("echo hi", "hi\n", 3)]);
    s.driver.settle_after_login(Duration::from_secs(30));
    assert!(s.driver.has_shell_integration());
    let result = s
        .driver
        .run("echo hi", Duration::from_secs(60), None)
        .unwrap();
    assert_eq!(result.mode, "osc133");
    assert_eq!((result.completed, result.exit_code), (true, Some(3)));
    assert_eq!(result.output, "hi\n");
    assert_eq!(s.session.sent_text(), "echo hi\n", "no probe, no wrapper");
}

#[test]
fn a_command_that_never_ends_times_out_and_stays_busy() {
    let s = scripted(Integration::Marker, &[]);
    s.driver.settle_after_login(Duration::from_secs(30));
    // From now on the shell echoes but never finishes.
    s.session.respond_with(|fake, bytes| {
        fake.emit_str(&String::from_utf8_lossy(bytes).replace('\n', "\r\n"))
    });
    let started = s.clock.now();
    let result = s
        .driver
        .run("sleep 100", Duration::from_secs(5), None)
        .unwrap();
    assert!(!result.completed);
    assert_eq!(result.exit_code, None);
    assert!(s.clock.now() - started >= Duration::from_secs(5));
    assert!(s.driver.is_busy());
    assert_eq!(
        s.driver.run("echo again", Duration::from_secs(1), None),
        Err(DriverError::Busy)
    );
}

#[test]
fn ctrl_c_finishes_an_interrupted_run_after_half_a_second_of_quiet() {
    let s = scripted(Integration::Marker, &[]);
    s.driver.settle_after_login(Duration::from_secs(30));
    s.session.respond_with(|fake, bytes| {
        if bytes == [0x03] {
            fake.emit_str("^C\r\ntest$ ");
        } else {
            fake.emit_str(&String::from_utf8_lossy(bytes).replace('\n', "\r\n"));
        }
    });
    let result = s
        .driver
        .run("sleep 100", Duration::from_secs(1), None)
        .unwrap();
    assert!(!result.completed && s.driver.is_busy());
    s.driver
        .send_keys(&[AgentKey::Named("ctrl-c".into())])
        .unwrap();
    let started = s.clock.now();
    let mut read = s
        .driver
        .read_output(None, Some(Duration::from_millis(1)), 1);
    while read.busy && s.clock.now() - started < Duration::from_secs(5) {
        s.clock.advance(Duration::from_millis(100));
        read = s
            .driver
            .read_output(None, Some(Duration::from_millis(1)), 1);
    }
    assert!(
        !read.busy,
        "an interrupted run ends once output is quiet at a prompt"
    );
    assert!(
        s.clock.now() - started < Duration::from_secs(2),
        "after 0.5 s of quiet, not the 1.5 s of quiet mode"
    );
    assert!(s
        .driver
        .run("echo again", Duration::from_secs(1), None)
        .is_ok());
}

#[test]
fn human_input_wins_for_up_to_two_seconds() {
    let s = scripted(Integration::Marker, &[("echo hi", "hi\n", 0)]);
    s.driver.settle_after_login(Duration::from_secs(30));
    let typed = s.clock.now();
    s.session.note_user_input(typed);
    s.session.sent.lock().unwrap().clear();
    let result = s
        .driver
        .run("echo hi", Duration::from_secs(60), None)
        .unwrap();
    assert!(result.completed);
    let waited = s.clock.now() - typed;
    assert!(waited >= Duration::from_secs(2), "{waited:?}");
    assert!(waited < Duration::from_secs(3), "{waited:?}");
}

#[test]
fn human_input_never_blocks_for_more_than_five_seconds() {
    let s = scripted(Integration::Marker, &[("echo hi", "hi\n", 0)]);
    s.driver.settle_after_login(Duration::from_secs(30));
    s.session.keep_typing(s.clock.clone());
    let started = s.clock.now();
    let result = s
        .driver
        .run("echo hi", Duration::from_secs(60), None)
        .unwrap();
    assert!(result.completed, "after the wait it proceeds anyway");
    let waited = s.clock.now() - started;
    assert!(
        waited >= Duration::from_secs(5) && waited < Duration::from_secs(6),
        "{waited:?}"
    );
}

#[test]
fn a_dead_session_ends_a_run_early_and_releases_busy() {
    let s = scripted(Integration::Marker, &[]);
    s.driver.settle_after_login(Duration::from_secs(30));
    s.session.respond_with(|fake, _| fake.exit());
    let result = s.driver.run("exit", Duration::from_secs(60), None).unwrap();
    assert!(!result.completed);
    assert!(!s.driver.is_busy());
    assert_eq!(
        s.driver.run("echo no", Duration::from_secs(1), None),
        Err(DriverError::NotConnected)
    );
}

#[test]
fn a_session_at_an_unknown_question_is_not_ready_but_a_quiet_banner_is() {
    let clock = Arc::new(ManualClock::new());
    let session = FakeSession::new();
    let driver = AgentDriver::new(
        session.clone() as Arc<dyn AgentSession>,
        Uuid::new_v4(),
        "A",
        clock.clone(),
    );
    assert!(!driver.is_ready(), "nothing has happened yet");
    session.emit_str("Enter verification code: ");
    clock.advance(Duration::from_secs(3));
    assert!(
        !driver.is_ready(),
        "a question ending in a colon is waiting for the user"
    );
    session.emit_str("123456\r\nWelcome to the box\r\n~ ");
    clock.advance(Duration::from_secs(3));
    assert!(driver.is_ready(), "quiet at no known question");
    session.emit_str("Password: ");
    assert!(driver.is_ready(), "ready latches");
}

#[test]
fn the_screen_read_waits_out_its_time_when_nothing_changes() {
    let s = scripted(Integration::Marker, &[]);
    s.driver.settle_after_login(Duration::from_secs(30));
    s.clock.advance(Duration::from_secs(1));
    assert!(s.driver.read_screen(None).changed);
    let started = s.clock.now();
    let idle = s.driver.read_screen(Some(Duration::from_secs(10)));
    assert!(!idle.changed);
    assert!(s.clock.now() - started >= Duration::from_secs(10));
    s.session.emit_str("new line\r\n");
    assert!(s.driver.read_screen(None).changed);
}

#[test]
fn a_typed_password_is_masked_in_output_and_screen_and_never_returned() {
    let s = scripted(Integration::Marker, &[]);
    s.driver.settle_after_login(Duration::from_secs(30));
    s.session.respond_with(|fake, bytes| {
        // A terminal that echoes, which a password prompt normally does not.
        fake.emit_str(&String::from_utf8_lossy(bytes).replace('\r', "\r\n"));
    });
    s.session.emit_str("[sudo] password for alice: ");
    s.driver.type_password("p4ss-word").unwrap();
    assert_eq!(s.session.sent_text().matches("p4ss-word\r").count(), 1);
    let out = s.driver.read_output(None, None, 65_536).output;
    assert!(
        out.contains(MASK_REPLACEMENT) && !out.contains("p4ss-word"),
        "{out:?}"
    );
    assert!(!s.driver.screen().rows.join("\n").contains("p4ss-word"));
    assert!(!s
        .driver
        .read_screen(None)
        .rows
        .join("\n")
        .contains("p4ss-word"));
}

#[test]
fn type_password_waits_for_a_prompt_that_is_still_arriving() {
    let s = scripted(Integration::Marker, &[]);
    s.driver.settle_after_login(Duration::from_secs(30));
    // No prompt within 500 ms of virtual time.
    assert_eq!(
        s.driver.type_password("x"),
        Err(DriverError::NoPasswordPrompt)
    );
    assert!(!s.session.sent_text().contains("x\r"));
}

#[test]
fn detaching_removes_the_tap_and_the_driven_flag() {
    let s = scripted(Integration::Marker, &[]);
    assert!(s.session.has_tap() && s.session.is_driven());
    s.driver.detach();
    assert!(!s.session.has_tap() && !s.session.is_driven());
    assert!(s.driver.is_detached() && !s.driver.is_busy());
    assert_eq!(
        s.driver.run("echo no", Duration::from_secs(1), None),
        Err(DriverError::Stopped)
    );
    s.driver.detach(); // idempotent
}

#[test]
fn dropping_a_driver_releases_the_session() {
    let clock = Arc::new(ManualClock::new());
    let session = FakeSession::new();
    let driver = AgentDriver::new(
        session.clone() as Arc<dyn AgentSession>,
        Uuid::new_v4(),
        "A",
        clock,
    );
    assert!(session.has_tap());
    drop(driver);
    assert!(!session.has_tap() && !session.is_driven());
}

#[test]
fn output_before_a_handoff_is_not_visible_to_the_agent() {
    let clock = Arc::new(ManualClock::new());
    let session = FakeSession::new();
    session.emit_str("secret history\r\ntest$ ");
    let driver = AgentDriver::new(
        session.clone() as Arc<dyn AgentSession>,
        Uuid::new_v4(),
        "A",
        clock,
    );
    assert_eq!(driver.read_output(None, None, 65_536).output, "");
    session.emit_str("after\r\n");
    assert_eq!(driver.read_output(None, None, 65_536).output, "after\n");
}

fn driven_bash() -> Driven {
    let session = spawn_bash();
    let driver = drive(&session, system_clock());
    driver.settle_after_login(T10);
    Driven { session, driver }
}

struct Guard(Arc<maine_agents::TermAgentSession>);

impl Drop for Guard {
    fn drop(&mut self) {
        end_shell(&self.0);
    }
}
