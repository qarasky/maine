//! The plain-text view of an agent session's output: PTY bytes with escape
//! sequences stripped, `\r` and backspace applied to the current line, kept in
//! a bounded ring. The bytes the terminal renders are untouched; this is a copy.
//!
//! The current, unfinished line is readable (prompts have no newline) and can
//! still be rewritten by `\r` until it ends. Masks (saved passwords that were
//! typed) are replaced when a line is rendered; passwords never span lines.
//!
//! Agents read by cursor, a position in the stream that only ever grows, and
//! grows whenever the text changes. For plain output it is the cleaned byte
//! count. A line redrawn in place (a progress bar or spinner behind `\r`) does
//! not get longer, so every change to it also advances the cursor: otherwise a
//! reader waiting for new output would see none, and a narrower redraw would
//! move the cursor backwards. Such a line therefore spans more cursor positions
//! than bytes; `anchors` record where cursor and byte offset diverge.

use std::collections::VecDeque;
use zeroize::Zeroize;

/// What a typed saved password is replaced with in everything an agent reads.
pub const MASK_REPLACEMENT: &str = "••••••";
/// A line is committed as if it had wrapped once it reaches this many characters.
pub const MAX_LINE_LENGTH: usize = 65_536;
/// Default size of the ring of committed lines, in bytes.
pub const DEFAULT_CAPACITY: usize = 2 * 1024 * 1024;
/// The most one result carries.
pub const MAX_RESULT_BYTES: usize = 65_536;

#[derive(Debug, Clone, Copy)]
struct Anchor {
    offset: i64,
    cursor: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Parser {
    Ground,
    Escape,
    EscapeIntermediate,
    Csi,
    Osc,
    OscEscape,
    Str,
    StrEscape,
}

/// What [`AgentOutput::text`] returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRead {
    pub text: String,
    pub cursor: i64,
    pub truncated: bool,
}

#[derive(Debug)]
pub struct AgentOutput {
    capacity: usize,
    /// Saved passwords typed into the session; replaced when a line is rendered.
    pub masks: Vec<String>,
    is_alternate_screen: bool,

    /// Committed lines; `ring_start` is the byte offset of its first byte.
    ring: Vec<u8>,
    ring_start: i64,
    /// Cursor at the end of the committed lines (where the current line starts).
    committed_cursor: i64,
    /// Cursor positions the current line has used: grows by the bytes of every
    /// character that changes it, so it never shrinks while the line is open.
    line_span: i64,
    /// Cursor and byte offset pairs, ascending. From each anchor on, both advance
    /// together until the next one. Every committed line that spanned more
    /// positions than bytes adds one at its end.
    anchors: VecDeque<Anchor>,
    line: Vec<char>,
    column: usize,
    parser: Parser,
    pending_utf8: Vec<u8>,
    expected_utf8: usize,
    csi_parameters: Vec<u8>,
}

impl Default for AgentOutput {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl Drop for AgentOutput {
    fn drop(&mut self) {
        for mask in &mut self.masks {
            mask.zeroize();
        }
    }
}

impl AgentOutput {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            masks: Vec::new(),
            is_alternate_screen: false,
            ring: Vec::new(),
            ring_start: 0,
            committed_cursor: 0,
            line_span: 0,
            anchors: VecDeque::from([Anchor {
                offset: 0,
                cursor: 0,
            }]),
            line: Vec::new(),
            column: 0,
            parser: Parser::Ground,
            pending_utf8: Vec::new(),
            expected_utf8: 0,
            csi_parameters: Vec::new(),
        }
    }

    pub fn is_alternate_screen(&self) -> bool {
        self.is_alternate_screen
    }

    /// Characters in the current, unfinished line.
    pub fn current_line_length(&self) -> usize {
        self.line.len()
    }

    /// The current end of the stream, including the current line.
    pub fn cursor(&self) -> i64 {
        self.committed_cursor + self.line_span.max(self.rendered_line().len() as i64)
    }

    pub fn append(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.consume(byte);
        }
        self.trim_ring();
    }

    /// Output after `since` with the default result size.
    pub fn text_since(&self, since: i64) -> TextRead {
        self.text(since, None, MAX_RESULT_BYTES)
    }

    /// Output after `since`, up to `up_to` when given. A cursor older than the
    /// ring starts at the ring and reports `truncated`; long results are cut in
    /// the middle. A cursor inside a line delivers that whole line again: `\r`
    /// may have rewritten it.
    pub fn text(&self, since: i64, up_to: Option<i64>, max_bytes: usize) -> TextRead {
        let rendered = self.rendered_line();
        let line_bytes = rendered.len() as i64;
        let stream_end = self.committed_cursor + self.line_span.max(line_bytes);
        let stop = up_to.map_or(stream_end, |limit| limit.min(stream_end));
        if since >= stop {
            return TextRead {
                text: String::new(),
                cursor: stop,
                truncated: false,
            };
        }
        let mut all = Vec::with_capacity(self.ring.len() + rendered.len());
        all.extend_from_slice(&self.ring);
        all.extend_from_slice(rendered.as_bytes());
        if stop < stream_end {
            let wanted = self.offset(stop, line_bytes) - self.ring_start;
            let mut cut = wanted.clamp(0, all.len() as i64) as usize;
            while cut < all.len() && all[cut] & 0xC0 == 0x80 {
                cut += 1;
            }
            all.truncate(cut);
        }
        let mut truncated = false;
        let start: i64 = if since < self.cursor_at_offset(self.ring_start) {
            truncated = true;
            0
        } else if since >= self.committed_cursor {
            self.ring.len() as i64 // the current line
        } else {
            self.offset(since, line_bytes) - self.ring_start
        };
        let mut start = start.clamp(0, all.len() as i64) as usize;
        if start < all.len() {
            while start > 0 && all[start - 1] != 0x0A {
                start -= 1;
            }
        }
        while start < all.len() && all[start] & 0xC0 == 0x80 {
            start += 1;
        }
        let text = String::from_utf8_lossy(&all[start..]).into_owned();
        let (text, cut) = middle_truncated(&text, max_bytes);
        TextRead {
            text,
            cursor: stop,
            truncated: truncated || cut,
        }
    }

    /// The byte offset for `cursor`. Inside a line that spans more positions
    /// than bytes it lands within that line, never in a later one.
    fn offset(&self, cursor: i64, line_bytes: i64) -> i64 {
        if cursor >= self.committed_cursor {
            return self.ring_start
                + self.ring.len() as i64
                + (cursor - self.committed_cursor).min(line_bytes);
        }
        let index = self.last_anchor(|anchor| anchor.cursor <= cursor);
        let anchor = self.anchors[index];
        let mut offset = anchor.offset + (cursor - anchor.cursor);
        if index + 1 < self.anchors.len() {
            offset = offset.min(self.anchors[index + 1].offset - 1);
        }
        offset
    }

    fn cursor_at_offset(&self, offset: i64) -> i64 {
        let anchor = self.anchors[self.last_anchor(|anchor| anchor.offset <= offset)];
        anchor.cursor + (offset - anchor.offset)
    }

    /// The last anchor matching `is_at_or_before` (a prefix of `anchors`); the first always does.
    fn last_anchor(&self, is_at_or_before: impl Fn(&Anchor) -> bool) -> usize {
        let mut low = 0;
        let mut high = self.anchors.len() - 1;
        while low < high {
            let middle = (low + high).div_ceil(2);
            if is_at_or_before(&self.anchors[middle]) {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        low
    }

    // MARK: - Parsing

    fn rendered_line(&self) -> String {
        let mut text: String = self.line.iter().collect();
        for mask in &self.masks {
            if !mask.is_empty() {
                text = text.replace(mask.as_str(), MASK_REPLACEMENT);
            }
        }
        text
    }

    fn consume(&mut self, byte: u8) {
        match self.parser {
            Parser::Ground => self.ground(byte),
            Parser::Escape => match byte {
                b'[' => {
                    self.csi_parameters.clear();
                    self.parser = Parser::Csi;
                }
                b']' => self.parser = Parser::Osc,
                b'P' | b'X' | b'^' | b'_' => self.parser = Parser::Str,
                0x20..=0x2F => self.parser = Parser::EscapeIntermediate,
                _ => self.parser = Parser::Ground,
            },
            Parser::EscapeIntermediate => self.parser = Parser::Ground,
            Parser::Csi => {
                if (0x40..=0x7E).contains(&byte) {
                    self.finish_csi(byte);
                    self.parser = Parser::Ground;
                } else if byte == 0x1B {
                    self.parser = Parser::Escape;
                } else {
                    self.csi_parameters.push(byte);
                }
            }
            Parser::Osc => {
                if byte == 0x07 {
                    self.parser = Parser::Ground;
                } else if byte == 0x1B {
                    self.parser = Parser::OscEscape;
                }
            }
            Parser::OscEscape => {
                self.parser = if byte == b'\\' {
                    Parser::Ground
                } else {
                    Parser::Osc
                }
            }
            Parser::Str => {
                if byte == 0x1B {
                    self.parser = Parser::StrEscape;
                }
            }
            Parser::StrEscape => {
                self.parser = if byte == b'\\' {
                    Parser::Ground
                } else {
                    Parser::Str
                }
            }
        }
    }

    fn ground(&mut self, byte: u8) {
        if byte >= 0x80 {
            self.utf8(byte);
            return;
        }
        self.pending_utf8.clear();
        match byte {
            0x1B => self.parser = Parser::Escape,
            0x0A => self.commit_line(),
            0x0D => self.column = 0,
            0x08 => self.column = self.column.saturating_sub(1),
            0x09 => self.put('\t'),
            0x20..=0x7E => self.put(byte as char),
            _ => {}
        }
    }

    fn utf8(&mut self, byte: u8) {
        if byte & 0xC0 == 0x80 {
            if self.pending_utf8.is_empty() {
                return;
            }
            self.pending_utf8.push(byte);
        } else {
            self.pending_utf8.clear();
            self.pending_utf8.push(byte);
            self.expected_utf8 = if byte >= 0xF0 {
                4
            } else if byte >= 0xE0 {
                3
            } else if byte >= 0xC0 {
                2
            } else {
                0
            };
            if self.expected_utf8 == 0 {
                self.pending_utf8.clear();
                return;
            }
        }
        if self.pending_utf8.len() != self.expected_utf8 {
            return;
        }
        let decoded = String::from_utf8_lossy(&self.pending_utf8).into_owned();
        for c in decoded.chars() {
            if c != '\u{FFFD}' {
                self.put(c);
            }
        }
        self.pending_utf8.clear();
    }

    fn put(&mut self, c: char) {
        // Output without newlines (minified files, base64) must stay bounded:
        // a full line is committed as if it had wrapped.
        if self.column >= MAX_LINE_LENGTH {
            self.commit_line();
        }
        let width = c.len_utf8() as i64;
        if self.column < self.line.len() {
            // Reprinting the same text (a shell redrawing its prompt) is no change.
            if self.line[self.column] != c {
                self.line[self.column] = c;
                self.line_span += width;
            }
        } else {
            self.line.push(c);
            self.line_span += width;
        }
        self.column += 1;
    }

    fn commit_line(&mut self) {
        let rendered = self.rendered_line();
        let length = rendered.len() as i64 + 1;
        let span = self.line_span.max(length - 1) + 1;
        self.ring.extend_from_slice(rendered.as_bytes());
        self.ring.push(b'\n');
        self.committed_cursor += span;
        if span != length {
            self.anchors.push_back(Anchor {
                offset: self.ring_start + self.ring.len() as i64,
                cursor: self.committed_cursor,
            });
        }
        self.line.clear();
        self.column = 0;
        self.line_span = 0;
    }

    fn finish_csi(&mut self, last: u8) {
        if (last != b'h' && last != b'l') || self.csi_parameters.first() != Some(&b'?') {
            return;
        }
        let modes = String::from_utf8_lossy(&self.csi_parameters[1..]).into_owned();
        if modes
            .split(';')
            .any(|mode| matches!(mode, "1049" | "1047" | "47"))
        {
            self.is_alternate_screen = last == b'h';
        }
    }

    fn trim_ring(&mut self) {
        if self.ring.len() <= self.capacity {
            return;
        }
        let mut drop = self.ring.len() - self.capacity;
        while drop < self.ring.len() && self.ring[drop] & 0xC0 == 0x80 {
            drop += 1;
        }
        self.ring.drain(..drop);
        self.ring_start += drop as i64;
        // Keep the anchor that still governs the ring's first byte.
        while self.anchors.len() > 1 && self.anchors[1].offset <= self.ring_start {
            self.anchors.pop_front();
        }
    }
}

/// Cuts `text` in the middle to at most `max_bytes`, leaving an
/// `…[N bytes omitted]…` marker. The flag says whether it was cut.
pub fn middle_truncated(text: &str, max_bytes: usize) -> (String, bool) {
    let bytes = text.as_bytes();
    if bytes.len() <= max_bytes {
        return (text.to_string(), false);
    }
    let marker_budget = format!("…[{} bytes omitted]…", bytes.len()).len();
    let keep = max_bytes.saturating_sub(marker_budget);
    let mut head_end = keep / 2;
    while head_end > 0 && bytes[head_end] & 0xC0 == 0x80 {
        head_end -= 1;
    }
    let mut tail_start = bytes.len() - (keep - keep / 2);
    while tail_start < bytes.len() && bytes[tail_start] & 0xC0 == 0x80 {
        tail_start += 1;
    }
    let omitted = tail_start - head_end;
    let head = String::from_utf8_lossy(&bytes[..head_end]);
    let tail = String::from_utf8_lossy(&bytes[tail_start..]);
    (format!("{head}…[{omitted} bytes omitted]…{tail}"), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cleaned(chunks: &[&str], capacity: usize) -> String {
        let mut output = AgentOutput::new(capacity);
        for chunk in chunks {
            output.append(chunk.as_bytes());
        }
        output.text_since(0).text
    }

    fn cleaned_default(chunks: &[&str]) -> String {
        cleaned(chunks, DEFAULT_CAPACITY)
    }

    #[test]
    fn escape_sequences_are_stripped() {
        assert_eq!(
            cleaned_default(&["\x1b[1;32mok\x1b[0m\r\n\x1b]0;title\x07done"]),
            "ok\ndone"
        );
        assert_eq!(cleaned_default(&["a\x1bP1;2|xyz\x1b\\b\x1b(Bc\n"]), "abc\n");
    }

    #[test]
    fn carriage_return_and_backspace_rewrite_the_current_line() {
        assert_eq!(cleaned_default(&["50%\r100%\n"]), "100%\n");
        assert_eq!(cleaned_default(&["abc\x08\x08X\n"]), "aXc\n");
        assert_eq!(cleaned_default(&["one\r\ntwo\r\n"]), "one\ntwo\n");
    }

    #[test]
    fn utf8_and_escapes_split_across_chunks_survive() {
        let bytes = "é✓\x1b[31mred".as_bytes();
        for index in 0..=bytes.len() {
            let mut output = AgentOutput::default();
            output.append(&bytes[..index]);
            output.append(&bytes[index..]);
            let text = output.text_since(0).text;
            assert_eq!(text, "é✓red", "split at {index}");
            assert!(!text.contains('\u{FFFD}'));
        }
    }

    #[test]
    fn cursors_are_monotonic_and_ring_is_bounded() {
        let mut output = AgentOutput::new(1_024);
        for _ in 0..5_000 {
            output.append(b"x\n");
        }
        assert_eq!(output.cursor(), 10_000);
        let all = output.text_since(0);
        assert!(all.truncated);
        assert!(all.text.len() <= 1_024);
        assert_eq!(all.cursor, 10_000);
        let tail = output.text_since(9_996);
        assert_eq!(tail.text, "x\nx\n");
        assert!(!tail.truncated);
        assert!(output.text_since(10_000).text.is_empty());
    }

    #[test]
    fn the_current_unfinished_line_is_readable() {
        let mut output = AgentOutput::default();
        output.append(b"[sudo] password for alice: ");
        assert_eq!(output.text_since(0).text, "[sudo] password for alice: ");
        assert_eq!(output.cursor(), 27);
    }

    #[test]
    fn long_results_are_cut_in_the_middle() {
        let (text, truncated) = middle_truncated(&"a".repeat(100_000), 65_536);
        assert!(truncated);
        assert!(text.len() <= 65_536);
        assert!(text.contains("…[") && text.contains("bytes omitted]…"));
        let (short, cut) = middle_truncated("short", 65_536);
        assert!(short == "short" && !cut);
    }

    #[test]
    fn alternate_screen_is_tracked() {
        let mut output = AgentOutput::default();
        output.append(b"\x1b[?1049h");
        assert!(output.is_alternate_screen());
        output.append(b"\x1b[?1049l");
        assert!(!output.is_alternate_screen());
        output.append(b"\x1b[?47h");
        assert!(output.is_alternate_screen());
    }

    #[test]
    fn masks_are_replaced_including_across_chunks() {
        let mut output = AgentOutput::default();
        output.masks = vec!["hunter2".into()];
        output.append(b"pw: hun");
        output.append(b"ter2\n");
        let text = output.text_since(0).text;
        assert!(text.contains("••••••"));
        assert!(!text.contains("hunter2"));
    }

    // MARK: - Final review fixes

    #[test]
    fn a_line_without_newlines_stays_bounded() {
        let mut output = AgentOutput::new(2 * 1024 * 1024);
        let chunk = vec![b'a'; 4_096];
        for _ in 0..256 {
            output.append(&chunk); // 1 MiB, no newline
        }
        assert!(output.current_line_length() <= MAX_LINE_LENGTH);
        // 16 full lines; the last stays open until the next character arrives.
        assert_eq!(
            output.cursor(),
            1_048_576 + 1_048_576 / MAX_LINE_LENGTH as i64 - 1
        );
        assert!(output.text_since(0).truncated); // longer than one result
    }

    #[test]
    fn reading_inside_a_rewritten_line_redelivers_the_whole_line() {
        let mut output = AgentOutput::default();
        output.append(b"Downloading 5%");
        let cursor = output.text_since(0).cursor;
        output.append(b"\rDownloading 100%\n");
        assert_eq!(output.text_since(cursor).text, "Downloading 100%\n");
        // A cursor at a committed boundary still reads only what is new.
        let end = output.cursor();
        output.append(b"next\n");
        assert_eq!(output.text_since(end).text, "next\n");
    }

    // MARK: - In-place redraws (progress bars, spinners)

    #[test]
    fn a_same_width_redraw_advances_the_cursor_and_is_readable() {
        let mut output = AgentOutput::default();
        output.append(b"$ job\n\r[##        ] 20%");
        let first = output.text_since(0);
        output.append(b"\r[#####     ] 50%");
        let next = output.text_since(first.cursor);
        assert!(next.cursor > first.cursor);
        assert_eq!(next.text, "[#####     ] 50%");
        // Nothing changed since: nothing new, cursor stays.
        assert!(output.text_since(next.cursor).text.is_empty());
        assert_eq!(output.text_since(next.cursor).cursor, next.cursor);
    }

    #[test]
    fn a_redraw_that_prints_the_same_text_is_not_new_output() {
        let mut output = AgentOutput::default();
        output.append(b"user@box:~$ ");
        let cursor = output.cursor();
        output.append(b"\ruser@box:~$ ");
        assert_eq!(output.cursor(), cursor);
    }

    #[test]
    fn a_narrower_redraw_never_moves_the_cursor_back() {
        let mut output = AgentOutput::default();
        output.append("\r━━━━━━━━ 80%".as_bytes());
        let before = output.cursor();
        output.append(b"\rdone 100%   ");
        let read = output.text_since(before);
        assert!(read.cursor > before);
        assert_eq!(read.text, "done 100%   ");
    }

    #[test]
    fn lines_after_a_redrawn_line_are_not_skipped() {
        let mut output = AgentOutput::default();
        output.append("━━━━━━━━━━━━━━━━ 10%".as_bytes());
        let held = output.cursor();
        output.append(b"\rok\x1b[K\na\nb\nc\nd\n");
        let read = output.text_since(held);
        assert!(read.text.ends_with("a\nb\nc\nd\n"), "{:?}", read.text);
        assert!(read.cursor > held);
        assert!(output.text_since(read.cursor).text.is_empty());
    }

    #[test]
    fn cursors_stay_monotonic_across_many_redraws_and_trimming() {
        let mut output = AgentOutput::new(256);
        let mut last = output.cursor();
        for step in 0..2_000 {
            output.append(format!("\r━━ {}%", step % 100).as_bytes());
            if step % 10 == 9 {
                output.append(format!("\nline {step}\n").as_bytes());
            }
            assert!(output.cursor() >= last);
            last = output.cursor();
        }
        let tail = output.text_since(0);
        assert!(tail.truncated);
        assert_eq!(tail.cursor, output.cursor());
        assert!(
            tail.text.ends_with("━━ 99%\nline 1999\n"),
            "{:?}",
            tail.text
        );
        assert!(output.text_since(tail.cursor).text.is_empty());
    }

    #[test]
    fn a_limit_inside_the_current_line_cuts_at_the_limit() {
        let mut output = AgentOutput::default();
        output.append(b"out\nab");
        let limit = output.cursor();
        output.append(b"cd\n");
        assert_eq!(
            output.text(0, Some(limit), MAX_RESULT_BYTES).text,
            "out\nab"
        );
    }

    // Extra coverage for the Rust port.

    #[test]
    fn invalid_utf8_is_dropped_not_replaced() {
        let mut output = AgentOutput::default();
        output.append(&[b'a', 0xFF, 0xC3, b'b', 0x80, b'c', b'\n']);
        assert_eq!(output.text_since(0).text, "abc\n");
    }

    #[test]
    fn masks_are_zeroized_on_drop_without_panicking() {
        let mut output = AgentOutput::default();
        output.masks.push("secret".into());
        drop(output);
    }
}
