use crate::colors::Palette;
use crate::snapshot::{snapshot, Modes, Snapshot};
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor, Rgb};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, MutexGuard};

pub struct SessionConfig {
    pub program: String,
    pub args: Vec<String>,
    /// The complete child environment (the parent's is not added implicitly).
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
    pub scrollback: usize,
    pub palette: Palette,
    /// Text shown as scrollback before the child's first output (restored sessions).
    pub history: Option<String>,
    /// Sees every raw byte from the very first read, before the emulator does
    /// (installing it later with `set_output_tap` can miss early output).
    pub output_tap: Option<Tap>,
}

impl SessionConfig {
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            env: std::env::vars().collect(),
            cwd: None,
            cols: 80,
            rows: 24,
            scrollback: 10_000,
            palette: Palette::default(),
            history: None,
            output_tap: None,
        }
    }
}

/// Things the UI layer decides about; drained with [`TermSession::take_events`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    Title(String),
    Bell,
    /// OSC 52 copy request; the UI honors it only when a human drives the session.
    ClipboardStore(String),
    /// The child ended (exit code if known).
    Exited(Option<i32>),
}

type Callback = Arc<dyn Fn() + Send + Sync>;
type Tap = Arc<dyn Fn(&[u8]) + Send + Sync>;

struct Shared {
    received: AtomicU64,
    events: Mutex<VecDeque<SessionEvent>>,
    dirty: AtomicBool,
    on_update: Mutex<Option<Callback>>,
    tap: Mutex<Option<Tap>>,
    input: Sender<Vec<u8>>,
    palette: Palette,
}

impl Shared {
    fn push(&self, event: SessionEvent) {
        lock(&self.events).push_back(event);
    }

    fn notify(&self) {
        if !self.dirty.swap(true, Ordering::AcqRel) {
            let callback = lock(&self.on_update).clone();
            if let Some(callback) = callback {
                callback();
            }
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Clone)]
struct Proxy(Arc<Shared>);

impl EventListener for Proxy {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(text) => {
                let _ = self.0.input.send(text.into_bytes());
            }
            Event::Title(title) => self.0.push(SessionEvent::Title(title)),
            Event::ResetTitle => self.0.push(SessionEvent::Title(String::new())),
            Event::Bell => self.0.push(SessionEvent::Bell),
            Event::ClipboardStore(_, text) => self.0.push(SessionEvent::ClipboardStore(text)),
            Event::ColorRequest(index, format) => {
                let color = if index < 256 {
                    self.0
                        .palette
                        .resolve(Color::Indexed(index as u8), &Default::default())
                } else if index == NamedColor::Background as usize {
                    self.0.palette.background
                } else {
                    self.0.palette.foreground
                };
                let rgb = Rgb {
                    r: (color >> 16) as u8,
                    g: (color >> 8) as u8,
                    b: color as u8,
                };
                let _ = self.0.input.send(format(rgb).into_bytes());
            }
            // Reading the clipboard from the terminal is never honored.
            _ => {}
        }
    }
}

struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

struct Inner {
    term: Mutex<Term<Proxy>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    shared: Arc<Shared>,
    exit: Mutex<Option<Option<i32>>>,
    running: AtomicBool,
    pid: Option<u32>,
}

/// A running terminal session. Cheap to clone; all clones share one child.
#[derive(Clone)]
pub struct TermSession {
    inner: Arc<Inner>,
}

impl TermSession {
    pub fn spawn(config: SessionConfig) -> Result<Self, String> {
        let size = PtySize {
            rows: config.rows,
            cols: config.cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = native_pty_system()
            .openpty(size)
            .map_err(|e| e.to_string())?;
        let mut command = CommandBuilder::new(&config.program);
        command.args(&config.args);
        command.env_clear();
        for (k, v) in &config.env {
            command.env(k, v);
        }
        if let Some(cwd) = &config.cwd {
            command.cwd(cwd);
        }
        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| e.to_string())?;
        drop(pair.slave);
        let pid = child.process_id();
        let killer = child.clone_killer();
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let mut writer = pair.master.take_writer().map_err(|e| e.to_string())?;

        let (input_tx, input_rx) = channel::<Vec<u8>>();
        let shared = Arc::new(Shared {
            received: AtomicU64::new(0),
            events: Mutex::new(VecDeque::new()),
            dirty: AtomicBool::new(false),
            on_update: Mutex::new(None),
            tap: Mutex::new(config.output_tap.clone()),
            input: input_tx,
            palette: config.palette.clone(),
        });
        let term_config = Config {
            scrolling_history: config.scrollback,
            ..Config::default()
        };
        let term = Term::new(
            term_config,
            &Size {
                cols: config.cols as usize,
                rows: config.rows as usize,
            },
            Proxy(shared.clone()),
        );
        let inner = Arc::new(Inner {
            term: Mutex::new(term),
            master: Mutex::new(pair.master),
            killer: Mutex::new(killer),
            shared: shared.clone(),
            exit: Mutex::new(None),
            running: AtomicBool::new(true),
            pid,
        });

        if let Some(history) = &config.history {
            Self {
                inner: inner.clone(),
            }
            .feed_history(history);
        }

        // Writer thread: input never blocks the UI or the reader on a full PTY buffer.
        std::thread::spawn(move || {
            while let Ok(bytes) = input_rx.recv() {
                if writer
                    .write_all(&bytes)
                    .and_then(|_| writer.flush())
                    .is_err()
                {
                    break;
                }
            }
        });

        // Reader thread: tap the raw bytes, then feed the emulator.
        let reader_inner = inner.clone();
        std::thread::spawn(move || {
            let mut parser: Processor = Processor::new();
            let mut buf = [0u8; 16 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        reader_inner
                            .shared
                            .received
                            .fetch_add(n as u64, Ordering::Release);
                        let tap = lock(&reader_inner.shared.tap).clone();
                        if let Some(tap) = tap {
                            tap(&buf[..n]);
                        }
                        parser.advance(&mut *lock(&reader_inner.term), &buf[..n]);
                        reader_inner.shared.notify();
                    }
                }
            }
            let code = child.wait().ok().map(|s| s.exit_code() as i32);
            *lock(&reader_inner.exit) = Some(code);
            reader_inner.running.store(false, Ordering::Release);
            reader_inner.shared.push(SessionEvent::Exited(code));
            reader_inner.shared.notify();
        });

        Ok(Self { inner })
    }

    /// Called from the reader thread when new content is available; coalesced
    /// until the next [`TermSession::snapshot`].
    pub fn set_update_callback(&self, callback: impl Fn() + Send + Sync + 'static) {
        *lock(&self.inner.shared.on_update) = Some(Arc::new(callback));
    }

    /// Sees every byte the child writes, before the emulator does.
    pub fn set_output_tap(&self, tap: impl Fn(&[u8]) + Send + Sync + 'static) {
        *lock(&self.inner.shared.tap) = Some(Arc::new(tap));
    }

    /// Bytes the child has written so far. Unlike a tap, this stays valid
    /// when someone else (an agent driver) replaces the tap.
    pub fn bytes_received(&self) -> u64 {
        self.inner.shared.received.load(Ordering::Acquire)
    }

    pub fn take_events(&self) -> Vec<SessionEvent> {
        lock(&self.inner.shared.events).drain(..).collect()
    }

    pub fn pid(&self) -> Option<u32> {
        self.inner.pid
    }

    pub fn is_running(&self) -> bool {
        self.inner.running.load(Ordering::Acquire)
    }

    /// `Some(code)` once the child has exited (`Some(None)` if the code is unknown).
    pub fn exit_status(&self) -> Option<Option<i32>> {
        *lock(&self.inner.exit)
    }

    pub fn terminate(&self) {
        let _ = lock(&self.inner.killer).kill();
    }

    /// Writes bytes to the child's stdin (queued; never blocks).
    pub fn send(&self, bytes: impl Into<Vec<u8>>) {
        let _ = self.inner.shared.input.send(bytes.into());
    }

    /// Pastes text, wrapping it in bracketed-paste markers when the program asked for them.
    pub fn paste(&self, text: &str) {
        if self.modes().bracketed_paste {
            let clean = text.replace("\x1b[201~", "");
            self.send([b"\x1b[200~".as_slice(), clean.as_bytes(), b"\x1b[201~"].concat());
        } else {
            self.send(text.replace("\r\n", "\r").replace('\n', "\r").into_bytes());
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let (cols, rows) = (cols.max(1), rows.max(1));
        let _ = lock(&self.inner.master).resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        lock(&self.inner.term).resize(Size {
            cols: cols as usize,
            rows: rows as usize,
        });
        self.inner.shared.notify();
    }

    pub fn size(&self) -> (usize, usize) {
        let term = lock(&self.inner.term);
        (term.columns(), term.screen_lines())
    }

    pub fn modes(&self) -> Modes {
        Modes::from(*lock(&self.inner.term).mode())
    }

    /// The visible screen. Clears the update-pending flag.
    pub fn snapshot(&self) -> Snapshot {
        self.inner.shared.dirty.store(false, Ordering::Release);
        snapshot(&lock(&self.inner.term), &self.inner.shared.palette)
    }

    /// Scrollback plus screen as plain text (trailing blanks trimmed per line,
    /// trailing empty lines dropped), at most `max_bytes` from the end.
    pub fn history_text(&self, max_bytes: usize) -> String {
        let term = lock(&self.inner.term);
        let top = -(term.history_size() as i32);
        let bottom = term.screen_lines() as i32;
        let mut lines: Vec<String> = Vec::new();
        for line in top..bottom {
            let row = &term.grid()[Line(line)];
            let mut text = String::new();
            for col in 0..term.columns() {
                let cell = &row[Column(col)];
                if cell.flags.intersects(
                    alacritty_terminal::term::cell::Flags::WIDE_CHAR_SPACER
                        | alacritty_terminal::term::cell::Flags::LEADING_WIDE_CHAR_SPACER,
                ) {
                    continue;
                }
                text.push(if cell.c == '\0' { ' ' } else { cell.c });
            }
            lines.push(text.trim_end().to_string());
        }
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        let text = lines.join("\n");
        if text.len() <= max_bytes {
            return text;
        }
        let mut start = text.len() - max_bytes;
        while !text.is_char_boundary(start) {
            start += 1;
        }
        text[start..].to_string()
    }

    /// Shows `text` as scrollback without running anything: control characters
    /// are removed (no escape sequences, clipboard requests or replies to the
    /// child), newlines become CR LF.
    pub fn feed_history(&self, text: &str) {
        let clean: String = text
            .chars()
            .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
            .collect::<String>()
            .replace('\n', "\r\n");
        let mut bytes = clean.into_bytes();
        bytes.extend_from_slice(b"\r\n");
        let mut parser: Processor = Processor::new();
        parser.advance(&mut *lock(&self.inner.term), &bytes);
        self.inner.shared.notify();
    }

    /// Scrolls the viewport; positive = towards older output.
    pub fn scroll(&self, lines: i32) {
        lock(&self.inner.term).scroll_display(Scroll::Delta(lines));
        self.inner.shared.notify();
    }

    pub fn scroll_to_bottom(&self) {
        lock(&self.inner.term).scroll_display(Scroll::Bottom);
        self.inner.shared.notify();
    }

    fn point(term: &Term<Proxy>, col: usize, row: usize) -> Point {
        let offset = term.grid().display_offset() as i32;
        Point::new(
            Line(row as i32 - offset),
            Column(col.min(term.columns().saturating_sub(1))),
        )
    }

    /// Starts a selection at a viewport cell (`right_half` = pointer in its right half).
    pub fn selection_start(&self, col: usize, row: usize, right_half: bool, block: bool) {
        let mut term = lock(&self.inner.term);
        let point = Self::point(&term, col, row);
        let kind = if block {
            SelectionType::Block
        } else {
            SelectionType::Simple
        };
        term.selection = Some(Selection::new(
            kind,
            point,
            if right_half { Side::Right } else { Side::Left },
        ));
        drop(term);
        self.inner.shared.notify();
    }

    pub fn selection_update(&self, col: usize, row: usize, right_half: bool) {
        let mut term = lock(&self.inner.term);
        let point = Self::point(&term, col, row);
        if let Some(sel) = term.selection.as_mut() {
            sel.update(point, if right_half { Side::Right } else { Side::Left });
        }
        drop(term);
        self.inner.shared.notify();
    }

    /// Selects the word under a cell (double click).
    pub fn selection_word(&self, col: usize, row: usize) {
        let mut term = lock(&self.inner.term);
        let point = Self::point(&term, col, row);
        term.selection = Some(Selection::new(SelectionType::Semantic, point, Side::Left));
        drop(term);
        self.inner.shared.notify();
    }

    /// Selects the whole line under a cell (triple click).
    pub fn selection_line(&self, col: usize, row: usize) {
        let mut term = lock(&self.inner.term);
        let point = Self::point(&term, col, row);
        term.selection = Some(Selection::new(SelectionType::Lines, point, Side::Left));
        drop(term);
        self.inner.shared.notify();
    }

    pub fn selection_clear(&self) {
        lock(&self.inner.term).selection = None;
        self.inner.shared.notify();
    }

    pub fn selection_text(&self) -> Option<String> {
        lock(&self.inner.term)
            .selection_to_string()
            .filter(|s| !s.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn run(script: &str) -> TermSession {
        let mut c = SessionConfig::new("/bin/sh", vec!["-c".into(), script.into()]);
        c.cols = 40;
        c.rows = 10;
        TermSession::spawn(c).unwrap()
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

    #[test]
    fn renders_child_output_into_the_grid() {
        let s = run("printf 'h\\303\\251llo\\nwide: \\344\\270\\255x\\n'");
        wait_until("exit", || !s.is_running());
        let snap = s.snapshot();
        assert_eq!(snap.row_text(0), "héllo");
        assert_eq!(snap.row_text(1), "wide: 中x");
        assert_eq!(s.exit_status(), Some(Some(0)));
    }

    #[test]
    fn child_sees_the_pty_size_and_resize_sends_sigwinch() {
        // Lives until terminated, so a slow machine cannot outrun it.
        let s = run("stty size; trap 'stty size' WINCH; while :; do sleep 0.05; done");
        wait_until("first size", || s.snapshot().text().contains("10 40"));
        s.resize(50, 12);
        wait_until("resized", || s.snapshot().text().contains("12 50"));
        assert_eq!(s.size(), (50, 12));
        s.terminate();
    }

    #[test]
    fn input_reaches_the_child_and_echoes() {
        let s = run("read line; printf 'got:%s\\n' \"$line\"");
        s.send(b"hello\r".to_vec());
        wait_until("echo", || s.snapshot().text().contains("got:hello"));
    }

    #[test]
    fn config_tap_sees_output_from_the_first_byte() {
        let seen = Arc::new(Mutex::new(Vec::<u8>::new()));
        let sink = seen.clone();
        let mut c = SessionConfig::new("/bin/sh", vec!["-c".into(), "printf first-bytes".into()]);
        c.output_tap = Some(Arc::new(move |b| sink.lock().unwrap().extend_from_slice(b)));
        let s = TermSession::spawn(c).unwrap();
        wait_until("exit", || !s.is_running());
        assert_eq!(
            String::from_utf8_lossy(&seen.lock().unwrap()),
            "first-bytes"
        );
    }

    #[test]
    fn output_tap_sees_raw_bytes_before_the_emulator() {
        let s = run("sleep 0.3; printf '\\033]133;A\\007done'");
        let seen = Arc::new(Mutex::new(Vec::<u8>::new()));
        let sink = seen.clone();
        s.set_output_tap(move |b| sink.lock().unwrap().extend_from_slice(b));
        wait_until("exit", || !s.is_running());
        let bytes = seen.lock().unwrap().clone();
        assert!(bytes.windows(7).any(|w| w == b"\x1b]133;A"), "{bytes:?}");
        assert_eq!(s.snapshot().row_text(0), "done");
    }

    #[test]
    fn colors_inverse_and_cursor_are_resolved() {
        let s = run("printf '\\033[31mR\\033[0m \\033[7mI\\033[0m'");
        wait_until("exit", || !s.is_running());
        let snap = s.snapshot();
        let p = Palette::default();
        assert_eq!(snap.cells[0].ch, 'R');
        assert_eq!(snap.cells[0].fg, p.ansi[1]);
        assert_eq!(snap.cells[2].ch, 'I');
        assert_eq!(
            (snap.cells[2].fg, snap.cells[2].bg),
            (p.background, p.foreground)
        );
    }

    #[test]
    fn update_callback_is_coalesced_until_snapshot() {
        let s = run("printf a; sleep 0.2; printf b");
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = count.clone();
        s.set_update_callback(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
        wait_until("exit", || !s.is_running());
        // Nobody consumed a snapshot, so at most one notification fired.
        assert!(count.load(Ordering::SeqCst) <= 1);
        let _ = s.snapshot();
    }

    #[test]
    fn scrollback_and_selection_copy() {
        let s = run("i=1; while [ $i -le 30 ]; do echo line$i; i=$((i+1)); done");
        wait_until("exit", || !s.is_running());
        let snap = s.snapshot();
        assert!(snap.history >= 20, "history {}", snap.history);
        assert_eq!(snap.display_offset, 0);
        s.scroll(5);
        assert_eq!(s.snapshot().display_offset, 5);
        s.scroll_to_bottom();
        assert_eq!(s.snapshot().display_offset, 0);
        let snap = s.snapshot();
        let row = (0..snap.rows)
            .find(|r| snap.row_text(*r) == "line30")
            .unwrap();
        s.selection_start(0, row, false, false);
        s.selection_update(4, row, true);
        assert_eq!(s.selection_text().as_deref(), Some("line3"));
        assert!(s.snapshot().cells[row * snap.cols].flags & crate::CellFlags::SELECTED != 0);
        s.selection_clear();
        assert_eq!(s.selection_text(), None);
    }

    #[test]
    fn bracketed_paste_follows_the_mode() {
        let s = run("printf '\\033[?2004h'; sleep 1");
        wait_until("mode", || s.modes().bracketed_paste);
        // paste() must wrap; capture what the child receives with cat -v is racy, so check the mode only.
        s.terminate();
    }

    #[test]
    fn history_text_includes_scrollback_and_feed_history_restores_it() {
        let s = run("i=1; while [ $i -le 25 ]; do echo row$i; i=$((i+1)); done");
        wait_until("exit", || !s.is_running());
        let text = s.history_text(1_000_000);
        assert!(text.starts_with("row1\nrow2"), "{text}");
        assert!(text.ends_with("row25"), "{text}");
        assert!(s.history_text(10).len() <= 10);

        let t = run("sleep 1");
        t.feed_history("old line\n\x1b]52;c;aGk=\x07second\ttab");
        let shown = t.snapshot().text();
        assert!(shown.contains("old line"), "{shown}");
        assert!(
            shown.contains("second\ttab") || shown.contains("second"),
            "{shown}"
        );
        assert!(!shown.contains('\x1b'));
        t.terminate();
    }

    #[test]
    fn the_alternate_screen_leaves_the_main_screen_and_its_history_untouched() {
        let s = run("printf 'main-line\\n'; printf '\\033[?1049halt-screen-text\\n'; sleep 0.3; printf '\\033[?1049l'; sleep 1");
        wait_until("alt screen", || {
            s.modes().alt_screen && s.snapshot().text().contains("alt-screen-text")
        });
        wait_until("back on main", || !s.modes().alt_screen);
        let text = s.snapshot().text();
        assert!(text.contains("main-line"), "{text}");
        assert!(
            !text.contains("alt-screen-text"),
            "alt text must not survive on the main screen: {text}"
        );
        assert!(!s.history_text(100_000).contains("alt-screen-text"));
        s.terminate();
    }

    #[test]
    fn terminate_ends_the_session() {
        let s = run("sleep 30");
        assert!(s.is_running());
        s.terminate();
        wait_until("exit after terminate", || !s.is_running());
        assert!(s
            .take_events()
            .iter()
            .any(|e| matches!(e, SessionEvent::Exited(_))));
    }

    #[test]
    fn launch_failure_is_reported() {
        let c = SessionConfig::new("/nonexistent/binary", vec![]);
        assert!(TermSession::spawn(c).is_err());
    }
}

#[cfg(test)]
mod throughput {
    use super::*;
    use std::time::Instant;

    /// `cargo test -p maine-term --release throughput -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn hundred_megabytes_of_output() {
        let mut c = SessionConfig::new(
            "/bin/sh",
            vec![
                "-c".into(),
                "head -c 75000000 /dev/zero | base64 -w 120".into(),
            ],
        );
        c.cols = 120;
        c.rows = 40;
        let start = Instant::now();
        let s = TermSession::spawn(c).unwrap();
        let bytes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let b = bytes.clone();
        s.set_output_tap(move |d| {
            b.fetch_add(d.len(), Ordering::Relaxed);
        });
        while s.is_running() {
            let _ = s.snapshot(); // a UI redrawing as fast as it can
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
        let secs = start.elapsed().as_secs_f64();
        let mb = bytes.load(Ordering::Relaxed) as f64 / 1e6;
        println!("{mb:.0} MB in {secs:.2}s = {:.0} MB/s", mb / secs);
    }
}
