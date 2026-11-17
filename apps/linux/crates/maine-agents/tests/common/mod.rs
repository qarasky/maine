//! Fakes and helpers shared by the integration tests: a scripted session (no
//! process, deterministic with a manual clock), a host that keeps real
//! `maine_term` sessions running `/bin/sh` instead of ssh, and wait helpers.
#![allow(dead_code)]

use maine_agents::{
    AgentAccess, AgentAuditEntry, AgentClient, AgentDriver, AgentError, AgentHost, AgentOutput,
    AgentSession, AgentSessionRef, AppleDate, Clock, OutputTap, ScreenText, SystemClock,
    TermAgentSession,
};
use maine_core::model::{Host, HostGroup, HostLayout};
use maine_term::{SessionConfig, TermSession};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use uuid::Uuid;
use zeroize::Zeroizing;

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Polls `condition` in real time; false if it never held.
pub fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    condition()
}

pub fn client(name: &str) -> AgentClient {
    AgentClient {
        id: Uuid::new_v4(),
        name: name.into(),
        created_at: AppleDate::now(),
    }
}

pub fn system_clock() -> Arc<dyn Clock> {
    Arc::new(SystemClock)
}

// MARK: - Real shells

/// Environment for a predictable interactive shell: a prompt that ends in `$`,
/// no startup files, no history.
pub fn shell_config(program: &str, args: &[&str]) -> SessionConfig {
    let mut config = SessionConfig::new(program, args.iter().map(|a| a.to_string()).collect());
    config.env.retain(|(key, _)| {
        !matches!(
            key.as_str(),
            "PS1" | "PS2" | "ENV" | "BASH_ENV" | "PROMPT_COMMAND" | "TERM"
        )
    });
    config.env.push(("PS1".into(), "test$ ".into()));
    config.env.push(("TERM".into(), "xterm-256color".into()));
    config.env.push(("HISTFILE".into(), "/dev/null".into()));
    config.cols = 100;
    config.rows = 30;
    config
}

/// An interactive `/bin/sh` (dash here) standing in for ssh. Dash prints its
/// `PS2` prompts after the echo, in front of the output: the harder case.
pub fn spawn_shell() -> Arc<TermAgentSession> {
    Arc::new(TermAgentSession::new(
        TermSession::spawn(shell_config("/bin/sh", &[])).expect("spawn /bin/sh"),
    ))
}

/// An interactive bash without startup files (for OSC 133 and PROMPT_COMMAND).
pub fn spawn_bash() -> Arc<TermAgentSession> {
    let config = shell_config("/usr/bin/bash", &["--norc", "--noprofile", "-i"]);
    Arc::new(TermAgentSession::new(
        TermSession::spawn(config).expect("spawn bash"),
    ))
}

/// A process that prints like ssh would, then stands in for the remote shell.
pub fn spawn_script(script: &str) -> Arc<TermAgentSession> {
    Arc::new(TermAgentSession::new(
        TermSession::spawn(shell_config("/bin/sh", &["-c", script])).expect("spawn"),
    ))
}

/// An interactive sh ignores SIGTERM-ish endings; end it the way a remote
/// shell ends: `exit`, then terminate for good measure.
pub fn end_shell(session: &TermAgentSession) {
    session.term().send(b"exit\n".to_vec());
    session.term().terminate();
}

pub fn drive(session: &Arc<TermAgentSession>, clock: Arc<dyn Clock>) -> Arc<AgentDriver> {
    AgentDriver::new(
        session.clone() as Arc<dyn AgentSession>,
        Uuid::new_v4(),
        "Test agent",
        clock,
    )
}

/// A shell with a driver attached; ends both when dropped, even if a test fails.
pub struct Driven {
    pub session: Arc<TermAgentSession>,
    pub driver: Arc<AgentDriver>,
}

impl Drop for Driven {
    fn drop(&mut self) {
        self.driver.detach();
        end_shell(&self.session);
    }
}

/// A connected shell with a driver attached, past its login prompt.
pub fn driven_shell() -> Driven {
    let session = spawn_shell();
    let driver = drive(&session, system_clock());
    driver.settle_after_login(Duration::from_secs(10));
    Driven { session, driver }
}

// MARK: - Scripted session

type Responder = Box<dyn FnMut(&FakeSession, &[u8]) + Send>;

/// A terminal with no process. What it prints is decided by a closure that
/// sees each input chunk; it feeds the driver's tap and a tiny screen model
/// (the same cleaning the agents read through).
pub struct FakeSession {
    tap: Mutex<Option<OutputTap>>,
    terminal: Mutex<AgentOutput>,
    running: AtomicBool,
    user_input: Mutex<Option<Instant>>,
    typing: Mutex<Option<Arc<dyn Clock>>>,
    driven: AtomicBool,
    pub sent: Mutex<Vec<u8>>,
    responder: Mutex<Option<Responder>>,
}

impl FakeSession {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            tap: Mutex::new(None),
            terminal: Mutex::new(AgentOutput::default()),
            running: AtomicBool::new(true),
            user_input: Mutex::new(None),
            typing: Mutex::new(None),
            driven: AtomicBool::new(false),
            sent: Mutex::new(Vec::new()),
            responder: Mutex::new(None),
        })
    }

    pub fn respond_with(&self, responder: impl FnMut(&FakeSession, &[u8]) + Send + 'static) {
        *lock(&self.responder) = Some(Box::new(responder));
    }

    /// The child writes `bytes`: the tap sees them, then the screen.
    pub fn emit(&self, bytes: &[u8]) {
        if let Some(tap) = lock(&self.tap).as_ref() {
            tap(bytes);
        }
        lock(&self.terminal).append(bytes);
    }

    pub fn emit_str(&self, text: &str) {
        self.emit(text.as_bytes());
    }

    pub fn exit(&self) {
        self.running.store(false, Ordering::Release);
    }

    pub fn note_user_input(&self, at: Instant) {
        *lock(&self.user_input) = Some(at);
    }

    /// A user who never stops typing: the last input is always "now" on `clock`.
    pub fn keep_typing(&self, clock: Arc<dyn Clock>) {
        *lock(&self.typing) = Some(clock);
    }

    pub fn is_driven(&self) -> bool {
        self.driven.load(Ordering::Acquire)
    }

    pub fn has_tap(&self) -> bool {
        lock(&self.tap).is_some()
    }

    pub fn sent_text(&self) -> String {
        String::from_utf8_lossy(&lock(&self.sent)).into_owned()
    }
}

impl AgentSession for FakeSession {
    fn send_input(&self, bytes: &[u8]) {
        lock(&self.sent).extend_from_slice(bytes);
        let responder = lock(&self.responder).take();
        if let Some(mut responder) = responder {
            responder(self, bytes);
            *lock(&self.responder) = Some(responder);
        }
    }

    fn set_output_tap(&self, tap: Option<OutputTap>) {
        *lock(&self.tap) = tap;
    }

    fn screen_text(&self) -> ScreenText {
        let text = lock(&self.terminal).text_since(0).text;
        let mut rows: Vec<String> = text
            .split('\n')
            .map(|row| row.trim_end().to_string())
            .collect();
        if rows.len() > 24 {
            rows.drain(..rows.len() - 24);
        }
        let cursor_row = rows.len().saturating_sub(1);
        let cursor_col = rows.last().map_or(0, |row| row.chars().count());
        ScreenText {
            rows,
            cursor_row,
            cursor_col,
        }
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    fn last_user_input(&self) -> Option<Instant> {
        if let Some(clock) = lock(&self.typing).as_ref() {
            return Some(clock.now());
        }
        *lock(&self.user_input)
    }

    fn set_agent_driven(&self, driven: bool) {
        self.driven.store(driven, Ordering::Release);
    }
}

/// How a scripted shell completes commands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Integration {
    /// Understands Maine's `printf` marker (bash, zsh, dash, ksh).
    Marker,
    /// No marker, no integration (fish and friends): the wrapper is just noise.
    Plain,
    /// Emits OSC 133 marks itself.
    Osc133,
}

pub const PROMPT: &str = "test$ ";

/// Where a plain shell prints its `PS2` continuation prompt for the wrapper's
/// second line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ps2 {
    /// Inline with the echo, as bash and zsh do through their line editors.
    Inline,
    /// After the whole echo, in front of the output, as dash and ash do.
    AfterEcho,
    /// Inside the echo of the second line, even inside the nonce: dash under
    /// load, when it reads the first line while the terminal still echoes the rest.
    MidEcho,
}

/// A shell that knows a few commands: `(output, exit code)` by command text.
pub fn scripted_shell(
    session: &Arc<FakeSession>,
    integration: Integration,
    commands: &[(&str, &str, i32)],
) {
    scripted_shell_with(session, integration, Ps2::Inline, commands);
}

pub fn scripted_shell_with(
    session: &Arc<FakeSession>,
    integration: Integration,
    ps2: Ps2,
    commands: &[(&str, &str, i32)],
) {
    let table: HashMap<String, (String, i32)> = commands
        .iter()
        .map(|(cmd, out, code)| (cmd.to_string(), (out.replace('\n', "\r\n"), *code)))
        .collect();
    let prompt = move |integration: Integration, code: i32| -> String {
        match integration {
            Integration::Osc133 => {
                format!("\x1b]133;D;{code}\x07\x1b]133;A\x07{PROMPT}\x1b]133;B\x07")
            }
            _ => PROMPT.to_string(),
        }
    };
    session.respond_with(move |fake, bytes| {
        let text = String::from_utf8_lossy(bytes).into_owned();
        if let Some(rest) = text.strip_prefix("{ ") {
            // Maine's wrapper: `{ <command>\n}; printf '\033]7770;<nonce>;%d\007' "$?"\n`
            let (command, tail) = rest
                .split_once("\n}; printf '\\033]7770;")
                .unwrap_or((rest, ""));
            let nonce = tail.split(';').next().unwrap_or("").to_string();
            let body = text.trim_end_matches('\n');
            let continuations = body.matches('\n').count();
            match ps2 {
                Ps2::Inline => fake.emit_str(&format!("{}\r\n", body.replace('\n', "\r\n> "))),
                Ps2::MidEcho => {
                    let (first, second) = body.split_once('\n').unwrap_or((body, ""));
                    let mut cut = second.len() / 2;
                    while !second.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    let tail_prompts = "> ".repeat(continuations.saturating_sub(1));
                    fake.emit_str(&format!(
                        "{first}\r\n{}> {}\r\n{tail_prompts}",
                        &second[..cut],
                        &second[cut..]
                    ));
                }
                Ps2::AfterEcho => {
                    fake.emit_str(&format!(
                        "{}\r\n{}",
                        body.replace('\n', "\r\n"),
                        "> ".repeat(continuations)
                    ));
                }
            }
            if integration == Integration::Marker {
                let (output, code) = table.get(command.trim_end()).cloned().unwrap_or_default();
                fake.emit_str(&output);
                fake.emit_str(&format!("\x1b]7770;{nonce};{code}\x07"));
                fake.emit_str(&prompt(integration, code));
            } else {
                fake.emit_str(&format!("sh: syntax error\r\n{}", prompt(integration, 2)));
            }
            return;
        }
        let command = text.trim_end_matches(['\n', '\r']);
        if let Some((output, code)) = table.get(command) {
            fake.emit_str(&format!("{command}\r\n"));
            if integration == Integration::Osc133 {
                fake.emit_str("\x1b]133;C\x07");
            }
            fake.emit_str(output);
            fake.emit_str(&prompt(integration, *code));
        } else {
            fake.emit_str(&text.replace('\n', "\r\n"));
        }
    });
}

// MARK: - Host

pub struct Record {
    pub id: Uuid,
    pub host_id: Uuid,
    pub name: String,
    pub terminal: Arc<TermAgentSession>,
    pub driver: Option<Arc<AgentDriver>>,
    pub former_agent: Option<String>,
    pub handed_off: bool,
    pub notice: Option<String>,
}

/// An app stand-in: a host list, rules, a vault flag, passwords and real
/// `/bin/sh` sessions where the app would run ssh.
pub struct TestHost {
    pub prod: HostGroup,
    pub lab: HostGroup,
    pub web: Host,
    pub db: Host,
    pub loose: Host,
    pub layout: Mutex<HostLayout>,
    pub access: Mutex<AgentAccess>,
    pub stored_access: Mutex<Option<AgentAccess>>,
    pub unlocked: AtomicBool,
    pub passwords: Mutex<HashMap<Uuid, String>>,
    pub audit: Mutex<Vec<AgentAuditEntry>>,
    pub diagnostics: Mutex<Vec<String>>,
    pub records: Mutex<Vec<Record>>,
    pub program: Mutex<(String, Vec<String>)>,
    pub extra_env: Mutex<Vec<(String, String)>>,
    pub clock: Arc<dyn Clock>,
}

impl TestHost {
    pub fn new() -> Arc<Self> {
        let prod = HostGroup::new("Prod");
        let lab = HostGroup::new("Lab");
        let mut web = Host::new("Web", "web.example");
        web.username = "deploy".into();
        web.group_id = Some(prod.id);
        let mut db = Host::new("DB", "db.example");
        db.group_id = Some(lab.id);
        let loose = Host::new("Loose", "loose.example");
        let layout = HostLayout::new(
            vec![prod.clone(), lab.clone()],
            vec![loose.clone(), web.clone(), db.clone()],
        );
        let passwords = [&web, &db, &loose]
            .iter()
            .map(|h| (h.id, format!("secret-{}", h.name)))
            .collect();
        Arc::new(Self {
            access: Mutex::new(AgentAccess::new([prod.id], [])),
            prod,
            lab,
            web,
            db,
            loose,
            layout: Mutex::new(layout),
            stored_access: Mutex::new(None),
            unlocked: AtomicBool::new(true),
            passwords: Mutex::new(passwords),
            audit: Mutex::new(Vec::new()),
            diagnostics: Mutex::new(Vec::new()),
            records: Mutex::new(Vec::new()),
            program: Mutex::new(("/bin/sh".into(), vec![])),
            extra_env: Mutex::new(vec![]),
            clock: system_clock(),
        })
    }

    pub fn set_program(&self, program: &str, args: &[&str]) {
        *lock(&self.program) = (
            program.to_string(),
            args.iter().map(|a| a.to_string()).collect(),
        );
    }

    pub fn set_access(&self, access: AgentAccess) {
        *lock(&self.access) = access;
    }

    pub fn set_unlocked(&self, unlocked: bool) {
        self.unlocked.store(unlocked, Ordering::Release);
    }

    fn spawn(&self) -> Result<Arc<TermAgentSession>, String> {
        let (program, args) = lock(&self.program).clone();
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut config = shell_config(&program, &args);
        config.env.extend(lock(&self.extra_env).iter().cloned());
        TermSession::spawn(config)
            .map(|term| Arc::new(TermAgentSession::new(term)))
            .map_err(|e| format!("Could not start {program}: {e}"))
    }

    /// A session the user opened (no agent).
    pub fn open_user_session(&self, host: &Host) -> Uuid {
        let terminal = self.spawn().expect("spawn");
        let id = Uuid::new_v4();
        let mut records = lock(&self.records);
        let name = format!("Session {}", records.len() + 1);
        records.push(Record {
            id,
            host_id: host.id,
            name,
            terminal,
            driver: None,
            former_agent: None,
            handed_off: false,
            notice: None,
        });
        id
    }

    pub fn ids(&self) -> Vec<Uuid> {
        lock(&self.records).iter().map(|r| r.id).collect()
    }

    pub fn with_record<T>(&self, id: Uuid, f: impl FnOnce(&mut Record) -> T) -> T {
        let mut records = lock(&self.records);
        f(records.iter_mut().find(|r| r.id == id).expect("record"))
    }

    pub fn driver(&self, id: Uuid) -> Option<Arc<AgentDriver>> {
        self.with_record(id, |r| r.driver.clone())
    }

    pub fn terminal(&self, id: Uuid) -> Arc<TermAgentSession> {
        self.with_record(id, |r| r.terminal.clone())
    }

    pub fn audit_entries(&self) -> Vec<AgentAuditEntry> {
        lock(&self.audit).clone()
    }

    pub fn tear_down(&self) {
        for record in lock(&self.records).drain(..) {
            if let Some(driver) = &record.driver {
                driver.detach();
            }
            end_shell(&record.terminal);
        }
    }

    fn make_ref(record: &Record) -> AgentSessionRef {
        let running = record.terminal.is_running();
        let exit = record.terminal.term().exit_status();
        let notice = record.notice.clone().or_else(|| match exit {
            Some(Some(code)) if code != 0 => {
                Some(format!("The connection closed with exit code {code}."))
            }
            _ => None,
        });
        AgentSessionRef {
            id: record.id,
            host_id: record.host_id,
            name: record.name.clone(),
            state_label: if running {
                "Connected".into()
            } else {
                "Closed".into()
            },
            is_live: running,
            notice,
            terminal: record.terminal.clone() as Arc<dyn AgentSession>,
            driver: record.driver.clone(),
        }
    }
}

impl AgentHost for TestHost {
    fn is_unlocked(&self) -> bool {
        self.unlocked.load(Ordering::Acquire)
    }

    fn layout(&self) -> HostLayout {
        lock(&self.layout).clone()
    }

    fn access(&self) -> AgentAccess {
        lock(&self.access).clone()
    }

    fn store_access(&self, access: &AgentAccess) -> Result<(), AgentError> {
        *lock(&self.stored_access) = Some(access.clone());
        *lock(&self.access) = access.clone();
        Ok(())
    }

    fn has_saved_password(&self, host_id: Uuid) -> bool {
        lock(&self.passwords).contains_key(&host_id)
    }

    fn saved_password(&self, host_id: Uuid) -> Option<Zeroizing<String>> {
        lock(&self.passwords)
            .get(&host_id)
            .map(|p| Zeroizing::new(p.clone()))
    }

    fn open_agent_session(
        &self,
        host: &Host,
        client: &AgentClient,
        _name: Option<&str>,
    ) -> Result<AgentSessionRef, String> {
        let terminal = self.spawn()?;
        let driver = AgentDriver::new(
            terminal.clone() as Arc<dyn AgentSession>,
            client.id,
            client.name.clone(),
            self.clock.clone(),
        );
        let id = Uuid::new_v4();
        let mut records = lock(&self.records);
        let name = format!("Session {}", records.len() + 1);
        records.push(Record {
            id,
            host_id: host.id,
            name,
            terminal,
            driver: Some(driver),
            former_agent: None,
            handed_off: false,
            notice: None,
        });
        Ok(Self::make_ref(records.last().unwrap()))
    }

    fn sessions(&self) -> Vec<AgentSessionRef> {
        lock(&self.records).iter().map(Self::make_ref).collect()
    }

    fn attach_agent(&self, session_id: Uuid, driver: Arc<AgentDriver>, handed_off: bool) {
        self.with_record(session_id, |record| {
            record.driver = Some(driver);
            record.handed_off = handed_off;
        });
    }

    fn end_agent_control(&self, session_id: Uuid) {
        let mut records = lock(&self.records);
        if let Some(record) = records.iter_mut().find(|r| r.id == session_id) {
            if let Some(driver) = record.driver.take() {
                driver.detach();
                record.former_agent = if record.handed_off {
                    None
                } else {
                    Some(driver.client_name().to_string())
                };
            }
        }
    }

    fn close_session(&self, session_id: Uuid) {
        let mut records = lock(&self.records);
        if let Some(index) = records.iter().position(|r| r.id == session_id) {
            let record = records.remove(index);
            end_shell(&record.terminal);
        }
    }

    fn record_audit(&self, entry: AgentAuditEntry) {
        lock(&self.audit).push(entry);
    }

    fn diagnostics(&self, event: &str, fields: &[(&str, String)]) {
        let line = fields
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ");
        lock(&self.diagnostics).push(format!("{event} {line}"));
    }
}

/// A short socket directory: `sun_path` holds at most 108 bytes.
pub fn short_socket_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("m-")
        .tempdir_in("/tmp")
        .expect("tempdir")
}
