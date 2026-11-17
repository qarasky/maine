//! Drives one agent session: types wrapped commands, keeps the cleaned output,
//! detects completion, and answers reads. Calls are blocking and may come from
//! several threads at once (the server runs each tool call on its own thread);
//! the PTY reader thread feeds [`AgentDriver::ingest`] through the session's
//! output tap. Waiting is done by short polls on the injected [`Clock`], which
//! keeps the state machine small and lets tests run without real sleeping.

use crate::clock::Clock;
use crate::completion::{
    is_password_prompt, looks_like_shell_prompt, CommandWrapper, CompletionDetector,
    Osc133Detector, Osc133Mark,
};
use crate::output::{middle_truncated, AgentOutput, MASK_REPLACEMENT, MAX_RESULT_BYTES};
use crate::session::AgentSession;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use uuid::Uuid;
use zeroize::Zeroizing;

const POLL: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverError {
    Busy,
    Paused,
    NotConnected,
    /// ssh is up but no remote shell is ready yet (still connecting, or
    /// waiting at a password, passphrase or host-key prompt).
    NotReady,
    /// `type_password` found no password prompt on the last screen line.
    NoPasswordPrompt,
    /// The caller's `should_stop` fired, or agent control ended; the handler
    /// decides what to report.
    Stopped,
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Busy => "a command is still running; use read_output, or send_keys ctrl-c",
            Self::Paused => "paused by the user",
            Self::NotConnected => "session is not connected",
            Self::NotReady => {
                "ssh is still connecting and the remote shell is not ready, so the command was not typed. \
                 Use read_screen to see where it is (it may wait at a password or host-key prompt: \
                 type_password or send_keys answers those), or wait and retry"
            }
            Self::Stopped => "not found",
            Self::NoPasswordPrompt => "no password prompt is showing; the password was not typed",
        })
    }
}

impl std::error::Error for DriverError {}

/// A key or literal text an agent sends to its session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentKey {
    Named(String),
    Text(String),
}

impl AgentKey {
    /// `text:<literal>`, or one of `enter`, `tab`, `esc`, `backspace`, `up`,
    /// `down`, `left`, `right`, `ctrl-a` .. `ctrl-z`.
    pub fn parse(raw: &str) -> Option<Self> {
        if let Some(text) = raw.strip_prefix("text:") {
            return Some(Self::Text(text.to_string()));
        }
        Self::named_bytes(raw).map(|_| Self::Named(raw.to_string()))
    }

    fn named_bytes(name: &str) -> Option<Vec<u8>> {
        Some(match name {
            "enter" => vec![0x0D],
            "tab" => vec![0x09],
            "esc" => vec![0x1B],
            "backspace" => vec![0x7F],
            "up" => b"\x1b[A".to_vec(),
            "down" => b"\x1b[B".to_vec(),
            "right" => b"\x1b[C".to_vec(),
            "left" => b"\x1b[D".to_vec(),
            _ => {
                let letter = name.strip_prefix("ctrl-")?;
                let mut chars = letter.chars();
                let (Some(c), None) = (chars.next(), chars.next()) else {
                    return None;
                };
                if !c.is_ascii_lowercase() {
                    return None;
                }
                vec![c as u8 - b'a' + 1]
            }
        })
    }

    pub fn bytes(&self) -> Vec<u8> {
        match self {
            Self::Named(name) => Self::named_bytes(name).unwrap_or_default(),
            Self::Text(text) => text.as_bytes().to_vec(),
        }
    }

    /// Ctrl-C and Ctrl-D end a running command without a completion marker.
    pub fn interrupts(&self) -> bool {
        matches!(self, Self::Named(name) if name == "ctrl-c" || name == "ctrl-d")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunResult {
    pub completed: bool,
    pub exit_code: Option<i32>,
    pub output: String,
    pub cursor: i64,
    pub truncated: bool,
    /// `marker`, `quiet`, `osc133`, or `unknown` before a mode was chosen.
    pub mode: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadResult {
    pub output: String,
    pub cursor: i64,
    pub busy: bool,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenResult {
    pub rows: Vec<String>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub alternate_screen: bool,
    /// Differs from what the previous `read_screen` returned (always true for the first).
    pub changed: bool,
    pub busy: bool,
}

impl ScreenResult {
    pub fn shows_same_as(&self, other: &ScreenResult) -> bool {
        self.rows == other.rows
            && self.cursor_row == other.cursor_row
            && self.cursor_col == other.cursor_col
            && self.alternate_screen == other.alternate_screen
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Unknown,
    Marker,
    Quiet,
    Osc133,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Marker => "marker",
            Self::Quiet => "quiet",
            Self::Osc133 => "osc133",
        }
    }
}

#[derive(Debug, Clone)]
struct PendingRun {
    start_cursor: i64,
    nonce: Option<String>,
    detector: Option<CompletionDetector>,
    end_cursor: Option<i64>,
    exit_code: Option<i32>,
    interrupted: bool,
    finished: bool,
    /// The marker probe, which must not release a `run`'s busy reservation.
    is_probe: bool,
    /// Where the command's own output starts, when the shell said so (`OSC 133;C`).
    output_start: Option<i64>,
    /// Lines the typed wrapper has after its first: the `PS2` prompts a plain
    /// shell prints while reading it.
    continuations: usize,
    /// How often `> ` occurs in the typed text itself (a redirection), so
    /// prompts inserted into its echo can be told from it.
    typed_prompt_lookalikes: usize,
}

impl PendingRun {
    fn new(start_cursor: i64) -> Self {
        Self {
            start_cursor,
            nonce: None,
            detector: None,
            end_cursor: None,
            exit_code: None,
            interrupted: false,
            finished: false,
            is_probe: false,
            output_start: None,
            continuations: 0,
            typed_prompt_lookalikes: 0,
        }
    }

    fn marked(start_cursor: i64, wrapper: &CommandWrapper, is_probe: bool) -> Self {
        Self {
            nonce: Some(wrapper.nonce.clone()),
            detector: Some(CompletionDetector::new(&wrapper.nonce)),
            is_probe,
            continuations: wrapper
                .typed_text
                .trim_end_matches('\n')
                .matches('\n')
                .count(),
            typed_prompt_lookalikes: wrapper.typed_text.matches("> ").count(),
            ..Self::new(start_cursor)
        }
    }
}

struct State {
    output: AgentOutput,
    mode: Mode,
    pending: Option<PendingRun>,
    last_output_at: Option<Instant>,
    /// Bumped by every PTY chunk; a waiting screen read re-renders only then.
    output_revision: u64,
    /// What the last `read_screen` returned, so the next one can wait for a change.
    last_screen: Option<ScreenResult>,
    shell_marks: Osc133Detector,
    /// The remote shell emitted `OSC 133;A` itself, so it has shell integration.
    has_shell_integration: bool,
    /// Set when agent control ends; every wait loop gives up.
    is_detached: bool,
    /// Latched once a remote shell is up; see [`AgentDriver::is_ready`].
    reached_shell: bool,
    is_busy: bool,
    is_active: bool,
}

pub struct AgentDriver {
    client_id: Uuid,
    client_name: String,
    session: Arc<dyn AgentSession>,
    clock: Arc<dyn Clock>,
    paused: AtomicBool,
    state: Mutex<State>,
}

impl fmt::Debug for AgentDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentDriver")
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

impl Drop for AgentDriver {
    fn drop(&mut self) {
        let detached = self
            .state
            .get_mut()
            .map(|st| st.is_detached)
            .unwrap_or(true);
        if !detached {
            self.session.set_output_tap(None);
            self.session.set_agent_driven(false);
        }
    }
}

impl AgentDriver {
    /// Attaches a driver to `session`: installs the output tap and marks the
    /// session as agent-driven. Call [`detach`](Self::detach) to end control.
    pub fn new(
        session: Arc<dyn AgentSession>,
        client_id: Uuid,
        client_name: impl Into<String>,
        clock: Arc<dyn Clock>,
    ) -> Arc<Self> {
        let driver = Arc::new(Self {
            client_id,
            client_name: client_name.into(),
            session: session.clone(),
            clock,
            paused: AtomicBool::new(false),
            state: Mutex::new(State {
                output: AgentOutput::default(),
                mode: Mode::Unknown,
                pending: None,
                last_output_at: None,
                output_revision: 0,
                last_screen: None,
                shell_marks: Osc133Detector::new(),
                has_shell_integration: false,
                is_detached: false,
                reached_shell: false,
                is_busy: false,
                is_active: false,
            }),
        });
        let weak = Arc::downgrade(&driver);
        session.set_output_tap(Some(Box::new(move |bytes| {
            if let Some(driver) = weak.upgrade() {
                driver.ingest(bytes);
            }
        })));
        session.set_agent_driven(true);
        driver
    }

    pub fn client_id(&self) -> Uuid {
        self.client_id
    }

    pub fn client_name(&self) -> &str {
        &self.client_name
    }

    pub fn session(&self) -> &Arc<dyn AgentSession> {
        &self.session
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Release);
    }

    /// A command is running (a `run` has not completed).
    pub fn is_busy(&self) -> bool {
        self.lock().is_busy
    }

    /// The agent is running a command or typing: the sidebar badge spins.
    pub fn is_active(&self) -> bool {
        self.lock().is_active
    }

    pub fn is_detached(&self) -> bool {
        self.lock().is_detached
    }

    /// The remote shell has emitted `OSC 133;A` itself (shell integration).
    pub fn has_shell_integration(&self) -> bool {
        self.lock().has_shell_integration
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn now(&self) -> Instant {
        self.clock.now()
    }

    // MARK: - Bytes from the PTY

    /// Bytes from the PTY, before the terminal renders them. Called from the
    /// session's output tap.
    pub fn ingest(&self, bytes: &[u8]) {
        let now = self.now();
        let mut st = self.lock();
        st.last_output_at = Some(now);
        st.output_revision = st.output_revision.wrapping_add(1);
        let marks = st.shell_marks.scan(bytes);
        if marks
            .iter()
            .any(|(mark, _)| *mark == Osc133Mark::PromptStart)
        {
            st.has_shell_integration = true;
        }
        if st.mode == Mode::Osc133 && !marks.is_empty() {
            Self::ingest_integrated(&mut st, bytes, &marks);
            return;
        }
        let hit = match st.pending.as_mut() {
            Some(run) if !run.finished => run
                .detector
                .as_mut()
                .and_then(|detector| detector.scan_for_end(bytes)),
            _ => None,
        };
        let Some((code, end)) = hit else {
            st.output.append(bytes);
            return;
        };
        st.output.append(&bytes[..end]);
        let end_cursor = st.output.cursor();
        let mut is_probe = false;
        if let Some(run) = st.pending.as_mut() {
            run.end_cursor = Some(end_cursor);
            run.exit_code = Some(code);
            run.finished = true;
            is_probe = run.is_probe;
        }
        if !is_probe {
            st.is_busy = false;
            st.is_active = false;
        }
        st.output.append(&bytes[end..]);
    }

    /// A shell with its own integration marks: `C` starts the command's
    /// output, the next `D` ends the command. Bytes still flow unchanged.
    fn ingest_integrated(st: &mut State, bytes: &[u8], marks: &[(Osc133Mark, usize)]) {
        let mut position = 0;
        for &(mark, end) in marks {
            st.output.append(&bytes[position..end]);
            position = end;
            let cursor = st.output.cursor();
            let Some(run) = st.pending.as_mut().filter(|run| !run.finished) else {
                continue;
            };
            match mark {
                Osc133Mark::OutputStart => {
                    if run.output_start.is_none() {
                        run.output_start = Some(cursor);
                    }
                }
                Osc133Mark::CommandDone(code) => {
                    run.end_cursor = Some(cursor);
                    run.exit_code = code;
                    run.finished = true;
                    st.is_busy = false;
                    st.is_active = false;
                }
                _ => {}
            }
        }
        st.output.append(&bytes[position..]);
    }

    // MARK: - Readiness

    /// A remote shell is up and can take commands. False while ssh is still
    /// connecting or authenticating, including at a password, passphrase or
    /// host-key prompt. Latches, so a long-running command never un-readies it.
    pub fn is_ready(&self) -> bool {
        if self.lock().reached_shell {
            return true;
        }
        if !self.session.is_running() {
            return false;
        }
        let screen = self.session.screen_text();
        let last = screen.last_non_empty_row();
        let (integrated, has_output, quiet) = {
            let st = self.lock();
            // The screen counts too: the child may have written before the tap was installed.
            (
                st.has_shell_integration,
                st.output.cursor() > 0 || !last.is_empty(),
                self.quiet_for(&st),
            )
        };
        let mut ready = integrated || looks_like_shell_prompt(last);
        // An unrecognised prompt style: output has gone quiet at no known question.
        if !ready
            && has_output
            && quiet >= Duration::from_millis(1500)
            && !Self::is_input_prompt(last)
        {
            ready = true;
        }
        if ready {
            self.lock().reached_shell = true;
        }
        ready
    }

    fn quiet_for(&self, st: &State) -> Duration {
        match st.last_output_at {
            Some(at) => self.now().saturating_duration_since(at),
            None => Duration::MAX,
        }
    }

    /// ssh waits for the user: a password, passphrase, MFA code or host-key answer.
    fn is_input_prompt(last: &str) -> bool {
        let line = last.trim().to_lowercase();
        is_password_prompt(last)
            || line.contains("(yes/no")
            || line.contains("passphrase")
            || line.ends_with(':')
            || line.ends_with('?')
    }

    /// After `open_session`: waits until the remote shell is ready, or until
    /// ssh stops at a prompt only the agent can answer, or the session dies.
    /// Then checks once whether this shell supports the completion marker.
    pub fn settle_after_login(&self, timeout: Duration) -> ReadResult {
        let deadline = self.now() + timeout;
        while self.now() < deadline && self.session.is_running() {
            if self.is_ready() {
                break;
            }
            // At a prompt: hand control back so the agent can answer it.
            let screen = self.session.screen_text();
            let last = screen.last_non_empty_row();
            let (has_output, quiet) = {
                let st = self.lock();
                (
                    st.output.cursor() > 0 || !last.is_empty(),
                    self.quiet_for(&st),
                )
            };
            if has_output && quiet >= Duration::from_millis(1500) && Self::is_input_prompt(last) {
                break;
            }
            self.clock.sleep(POLL);
        }
        let login = self.lock().output.text_since(0);
        if self.is_ready() && self.lock().mode == Mode::Unknown {
            self.choose_mode();
        }
        ReadResult {
            output: login.text,
            cursor: login.cursor,
            busy: self.is_busy(),
            truncated: login.truncated,
        }
    }

    // MARK: - Tools

    /// Types `command` and waits for it. `should_stop` is polled while waiting
    /// (access revoked mid-run); when it fires, `Stopped` is returned.
    pub fn run(
        &self,
        command: &str,
        timeout: Duration,
        should_stop: Option<&dyn Fn() -> bool>,
    ) -> Result<RunResult, DriverError> {
        self.ensure_usable()?;
        // Nothing is typed before the remote shell is up: a command typed into
        // a half-open connection lands in a password prompt or is lost.
        if !self.is_ready() {
            return Err(DriverError::NotReady);
        }
        {
            // Reserve before anything can wait, so a parallel run sees busy.
            let mut st = self.lock();
            if st.is_busy {
                return Err(DriverError::Busy);
            }
            st.is_busy = true;
            st.is_active = true;
        }
        let prepared = self.prepare_run();
        if let Err(error) = prepared {
            let mut st = self.lock();
            st.is_busy = false;
            st.is_active = false;
            return Err(error);
        }
        let typed = {
            let mut st = self.lock();
            let start = st.output.cursor();
            if st.mode == Mode::Marker {
                let wrapper = CommandWrapper::new(command);
                st.pending = Some(PendingRun::marked(start, &wrapper, false));
                wrapper.typed_text
            } else {
                // Quiet-period and shell-integration modes type the command as is.
                st.pending = Some(PendingRun::new(start));
                format!("{command}\n")
            }
        };
        self.session.send_input(typed.as_bytes());
        let deadline = self.now() + timeout;
        while self.now() < deadline {
            self.refresh_completion();
            let (finished, detached) = {
                let st = self.lock();
                (
                    st.pending.as_ref().is_some_and(|run| run.finished),
                    st.is_detached,
                )
            };
            if finished || !self.session.is_running() {
                break;
            }
            if detached || should_stop.is_some_and(|stop| stop()) {
                return Err(DriverError::Stopped);
            }
            self.clock.sleep(POLL);
        }
        self.refresh_completion();
        let running = self.session.is_running();
        let mut st = self.lock();
        if !running && !st.pending.as_ref().is_some_and(|run| run.finished) {
            st.is_busy = false;
            st.is_active = false;
        }
        Ok(Self::run_result(&st))
    }

    /// Everything `run` does between reserving the session and typing.
    fn prepare_run(&self) -> Result<(), DriverError> {
        self.wait_for_human_pause();
        self.ensure_usable()?;
        if self.lock().mode == Mode::Unknown {
            self.choose_mode();
        }
        if self.lock().is_detached {
            return Err(DriverError::Stopped);
        }
        Ok(())
    }

    pub fn read_output(
        &self,
        since: Option<i64>,
        wait: Option<Duration>,
        max_bytes: usize,
    ) -> ReadResult {
        let since = since.unwrap_or(0);
        if let Some(wait) = wait {
            let deadline = self.now() + wait;
            let was_busy = self.is_busy();
            while self.now() < deadline {
                self.refresh_completion();
                let (cursor, busy, detached) = {
                    let st = self.lock();
                    (st.output.cursor(), st.is_busy, st.is_detached)
                };
                if cursor > since || (was_busy && !busy) || !self.session.is_running() || detached {
                    break;
                }
                self.clock.sleep(POLL);
            }
        }
        self.refresh_completion();
        let st = self.lock();
        let text = st.output.text(since, None, max_bytes);
        ReadResult {
            output: text.text,
            cursor: text.cursor,
            busy: st.is_busy,
            truncated: text.truncated,
        }
    }

    /// The visible screen now, with saved-password masks applied.
    pub fn screen(&self) -> ScreenResult {
        let screen = self.session.screen_text();
        let st = self.lock();
        let rows = screen
            .rows
            .iter()
            .map(|row| {
                st.output
                    .masks
                    .iter()
                    .filter(|mask| !mask.is_empty())
                    .fold(row.clone(), |text, mask| {
                        text.replace(mask.as_str(), MASK_REPLACEMENT)
                    })
            })
            .collect();
        ScreenResult {
            rows,
            cursor_row: screen.cursor_row,
            cursor_col: screen.cursor_col,
            alternate_screen: st.output.is_alternate_screen(),
            changed: true,
            busy: false,
        }
    }

    /// `read_screen`: the screen now, or with `wait`, once it differs from
    /// what the previous read returned (or the command ends, or time runs out).
    /// Multi-line progress that moves the cursor up only reads right here.
    pub fn read_screen(&self, wait: Option<Duration>) -> ScreenResult {
        let mut current = self.screen();
        let last = self.lock().last_screen.clone();
        if let (Some(wait), Some(last)) = (wait, last.as_ref()) {
            if current.shows_same_as(last) {
                let deadline = self.now() + wait;
                let was_busy = self.is_busy();
                let mut seen = self.lock().output_revision;
                while self.now() < deadline {
                    self.clock.sleep(POLL);
                    self.refresh_completion();
                    let (busy, detached, revision) = {
                        let st = self.lock();
                        (st.is_busy, st.is_detached, st.output_revision)
                    };
                    if (was_busy && !busy) || !self.session.is_running() || detached {
                        break;
                    }
                    if revision == seen {
                        continue;
                    }
                    seen = revision;
                    if !self.screen().shows_same_as(last) {
                        break;
                    }
                }
                current = self.screen();
            }
        }
        current.changed = last
            .as_ref()
            .is_none_or(|last| !current.shows_same_as(last));
        let mut st = self.lock();
        current.busy = st.is_busy;
        st.last_screen = Some(current.clone());
        current
    }

    pub fn send_keys(&self, keys: &[AgentKey]) -> Result<(), DriverError> {
        self.ensure_usable()?;
        self.wait_for_human_pause();
        self.ensure_usable()?;
        self.lock().is_active = true;
        for key in keys {
            self.session.send_input(&key.bytes());
            if key.interrupts() {
                let mut st = self.lock();
                if st.is_busy {
                    if let Some(run) = st.pending.as_mut() {
                        run.interrupted = true;
                    }
                }
            }
        }
        let mut st = self.lock();
        if !st.is_busy {
            st.is_active = false;
        }
        Ok(())
    }

    /// Types `password` plus Enter, but only when the last non-empty screen
    /// line is a password prompt (see [`is_password_prompt`]), so it can never
    /// land in a shell line or an editor. The password joins the mask list
    /// first, so no later output or screen read can reveal it. It is never
    /// returned, logged or put in an argument list.
    pub fn type_password(&self, password: &str) -> Result<(), DriverError> {
        self.ensure_usable()?;
        self.wait_for_human_pause();
        self.ensure_usable()?;
        // The prompt may still be on its way from the PTY to the screen.
        let deadline = self.now() + Duration::from_millis(500);
        while !self.shows_password_prompt() && self.now() < deadline {
            self.clock.sleep(POLL);
        }
        if !self.shows_password_prompt() {
            return Err(DriverError::NoPasswordPrompt);
        }
        {
            let mut st = self.lock();
            if !password.is_empty() && !st.output.masks.iter().any(|mask| mask == password) {
                st.output.masks.push(password.to_string());
            }
            st.is_active = true;
        }
        let mut typed = Zeroizing::new(Vec::with_capacity(password.len() + 1));
        typed.extend_from_slice(password.as_bytes());
        typed.push(b'\r');
        self.session.send_input(&typed);
        let mut st = self.lock();
        if !st.is_busy {
            st.is_active = false;
        }
        Ok(())
    }

    fn shows_password_prompt(&self) -> bool {
        let screen = self.session.screen_text();
        is_password_prompt(screen.last_non_empty_row())
    }

    /// Ends this driver's control; the session stays open.
    pub fn detach(&self) {
        {
            let mut st = self.lock();
            if st.is_detached {
                return;
            }
            st.is_detached = true;
            st.is_busy = false;
            st.is_active = false;
            st.pending = None;
        }
        self.session.set_output_tap(None);
        self.session.set_agent_driven(false);
    }

    // MARK: - Internals

    fn ensure_usable(&self) -> Result<(), DriverError> {
        if self.is_paused() {
            return Err(DriverError::Paused);
        }
        if !self.session.is_running() {
            return Err(DriverError::NotConnected);
        }
        Ok(())
    }

    /// Agent input waits while the user typed in the last 2 s, at most 5 s.
    fn wait_for_human_pause(&self) {
        let deadline = self.now() + Duration::from_secs(5);
        while self.now() < deadline {
            match self.session.last_user_input() {
                Some(typed)
                    if self.now().saturating_duration_since(typed) < Duration::from_secs(2) =>
                {
                    self.clock.sleep(Duration::from_millis(100));
                }
                _ => break,
            }
        }
    }

    /// Quiet-mode completion and interrupted runs: no marker will arrive, so
    /// finish once output is quiet and the last line looks like a prompt.
    fn refresh_completion(&self) {
        let mut st = self.lock();
        let Some(run) = st.pending.as_ref().filter(|run| !run.finished) else {
            return;
        };
        let needed = if run.interrupted {
            Some(Duration::from_millis(500))
        } else if st.mode == Mode::Quiet {
            Some(Duration::from_millis(1500))
        } else {
            None
        };
        let Some(needed) = needed else { return };
        if self.quiet_for(&st) < needed || st.output.cursor() <= run.start_cursor {
            return;
        }
        if !looks_like_shell_prompt(&Self::last_line(&st)) {
            return;
        }
        let end = st.output.cursor();
        if let Some(run) = st.pending.as_mut() {
            run.finished = true;
            run.end_cursor = Some(end);
        }
        st.is_busy = false;
        st.is_active = false;
    }

    fn last_line(st: &State) -> String {
        let text = st
            .output
            .text((st.output.cursor() - 512).max(0), None, MAX_RESULT_BYTES)
            .text;
        text.rsplit('\n').next().unwrap_or("").to_string()
    }

    fn run_result(st: &State) -> RunResult {
        let Some(run) = st.pending.as_ref() else {
            return RunResult {
                completed: false,
                exit_code: None,
                output: String::new(),
                cursor: st.output.cursor(),
                truncated: false,
                mode: st.mode.name(),
            };
        };
        let finish = |text: String| {
            let (output, truncated) = middle_truncated(&text, MAX_RESULT_BYTES);
            RunResult {
                completed: run.finished && !run.interrupted,
                exit_code: run.exit_code,
                output,
                cursor: run.end_cursor.unwrap_or_else(|| st.output.cursor()),
                truncated,
                mode: st.mode.name(),
            }
        };
        if st.mode == Mode::Osc133 {
            // The shell marked where the output starts (when it emits `C`);
            // otherwise skip the echoed command line. It ends at `D`, so no
            // trailing prompt to trim.
            let mut text = st
                .output
                .text(
                    run.output_start.unwrap_or(run.start_cursor),
                    run.end_cursor,
                    usize::MAX,
                )
                .text;
            if run.output_start.is_none() {
                if let Some(newline) = text.find('\n') {
                    text = text[newline + 1..].to_string();
                }
            }
            return finish(text);
        }
        let mut text = st
            .output
            .text(run.start_cursor, run.end_cursor, usize::MAX)
            .text;
        if let Some(nonce) = &run.nonce {
            // Drop the echoed prompt and wrapper: everything through the line
            // that carries this command's nonce.
            let needle = format!("7770;{nonce};");
            let lines: Vec<&str> = text.split('\n').collect();
            // The line that carries this command's nonce. A plain shell may
            // have printed a `PS2` prompt (`> `) in the middle of the echo,
            // even inside the nonce, so a line also matches without its `> `s.
            let echo = lines.iter().rposition(|line| {
                line.contains(&needle) || line.replace("> ", "").contains(&needle)
            });
            if let Some(echo) = echo {
                let mut rest = lines[echo + 1..].join("\n");
                // dash and ash echo the typed text first and only then print
                // their `PS2` prompts, so those land in front of the output;
                // bash and zsh print them inside the echo. Drop as many
                // leading `> ` as the wrapper's continuation lines still lack
                // inside the echo, never more, so output that really begins
                // with `> ` survives.
                let echoed = lines[..=echo].join("\n");
                let region = echoed
                    .find("{ ")
                    .map_or(echoed.as_str(), |start| &echoed[start..]);
                let inside = region
                    .matches("> ")
                    .count()
                    .saturating_sub(run.typed_prompt_lookalikes);
                for _ in 0..run.continuations.saturating_sub(inside) {
                    match rest.strip_prefix("> ") {
                        Some(stripped) => rest = stripped.to_string(),
                        None => break,
                    }
                }
                text = rest;
            }
        } else if let Some(newline) = text.find('\n') {
            text = text[newline + 1..].to_string(); // the echoed command line
        }
        if run.finished && run.nonce.is_none() {
            if let Some(newline) = text.rfind('\n') {
                text.truncate(newline); // the prompt that followed
            }
        }
        finish(text)
    }

    /// A remote shell that already emits `OSC 133` needs no wrapper; any other
    /// is probed once for Maine's own marker, else it falls back to quiet mode.
    fn choose_mode(&self) {
        let integrated = self.lock().has_shell_integration;
        if integrated {
            self.lock().mode = Mode::Osc133;
        } else {
            self.probe_marker();
        }
    }

    /// Types a wrapped `true` and waits up to 2 s for its marker.
    fn probe_marker(&self) {
        let wrapper = CommandWrapper::new("true");
        {
            let mut st = self.lock();
            let start = st.output.cursor();
            st.pending = Some(PendingRun::marked(start, &wrapper, true));
        }
        self.session.send_input(wrapper.typed_text.as_bytes());
        let deadline = self.now() + Duration::from_secs(2);
        while self.now() < deadline
            && !self.lock().pending.as_ref().is_some_and(|run| run.finished)
            && self.session.is_running()
        {
            self.clock.sleep(POLL);
        }
        {
            let mut st = self.lock();
            st.mode = if st.pending.as_ref().is_some_and(|run| run.finished) {
                Mode::Marker
            } else {
                Mode::Quiet
            };
            st.pending = None;
        }
        // Let the prompt after the probe arrive before the next command.
        let settle = self.now() + Duration::from_millis(300);
        while self.now() < settle {
            self.clock.sleep(POLL);
        }
    }
}
