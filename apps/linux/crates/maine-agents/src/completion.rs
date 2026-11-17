//! Knowing that an agent's command finished: Maine's own per-command marker,
//! the shell-integration marks (`OSC 133`) a remote shell may already emit,
//! and the prompt heuristics. Everything here is pure and works on raw PTY bytes.

use rand::RngCore;
use std::collections::VecDeque;

/// The text typed for an agent's `run`: the command in a `{ ... }` group, then
/// a `printf` that emits `ESC ] 7770 ; <nonce> ; <exit> BEL`. The terminal
/// ignores that OSC; the detector finds it in the raw PTY bytes. The echoed
/// line contains the characters `\033`, never an ESC byte, so the echo can't
/// match, and a remote program can't guess the per-command nonce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandWrapper {
    pub nonce: String,
    pub typed_text: String,
}

impl CommandWrapper {
    pub fn new(command: &str) -> Self {
        Self::with_nonce(command, Self::make_nonce())
    }

    pub fn with_nonce(command: &str, nonce: String) -> Self {
        let typed_text = format!("{{ {command}\n}}; printf '\\033]7770;{nonce};%d\\007' \"$?\"\n");
        Self { nonce, typed_text }
    }

    /// 16 random bytes as lowercase hex.
    pub fn make_nonce() -> String {
        let mut bytes = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Streams raw PTY bytes and reports the exit code once, when the marker for
/// its nonce has fully arrived. The marker may be split across chunks.
#[derive(Debug, Clone)]
pub struct CompletionDetector {
    prefix: Vec<u8>,
    window: VecDeque<u8>,
    finished: bool,
}

impl CompletionDetector {
    pub fn new(nonce: &str) -> Self {
        Self {
            prefix: format!("\x1b]7770;{nonce};").into_bytes(),
            window: VecDeque::new(),
            finished: false,
        }
    }

    pub fn scan(&mut self, bytes: &[u8]) -> Option<i32> {
        self.scan_for_end(bytes).map(|(code, _)| code)
    }

    /// Like [`scan`](Self::scan), also returning the index just past the
    /// marker's BEL, so output after the marker (the next prompt) can be told apart.
    pub fn scan_for_end(&mut self, bytes: &[u8]) -> Option<(i32, usize)> {
        if self.finished {
            return None;
        }
        let limit = self.prefix.len() + 12;
        for (index, &byte) in bytes.iter().enumerate() {
            self.window.push_back(byte);
            while self.window.len() > limit {
                self.window.pop_front();
            }
            if byte != 0x07 {
                continue;
            }
            if let Some(code) = self.matches() {
                self.finished = true;
                return Some((code, index + 1));
            }
        }
        None
    }

    /// The window ends in BEL: does it end in `prefix digits BEL`?
    fn matches(&self) -> Option<i32> {
        let window: Vec<u8> = self.window.iter().copied().collect();
        let mut digits_start = window.len().checked_sub(1)?; // index of the BEL
        while digits_start > 0 && window[digits_start - 1].is_ascii_digit() {
            digits_start -= 1;
        }
        let digits = &window[digits_start..window.len() - 1];
        if digits.is_empty() || digits.len() > 10 || digits_start < self.prefix.len() {
            return None;
        }
        if window[digits_start - self.prefix.len()..digits_start] != self.prefix[..] {
            return None;
        }
        std::str::from_utf8(digits).ok()?.parse::<i32>().ok()
    }
}

/// A line that ends like an interactive shell prompt (`$`, `#`, `%`, `>`),
/// used only by the quiet-period fallback for shells without the marker, and
/// to decide whether a session may be handed to an agent.
pub fn looks_like_shell_prompt(line: &str) -> bool {
    let trimmed = line.trim();
    let mut chars = trimmed.chars().rev();
    let Some(last) = chars.next() else {
        return false;
    };
    if !"$#%>".contains(last) || trimmed.chars().count() <= 1 {
        return false;
    }
    if last == '%' && chars.next().is_some_and(|previous| previous.is_numeric()) {
        return false;
    }
    true
}

/// A prompt asking for an account password (`sudo`, `su`, ssh), not a
/// passphrase, one-time code or verification code. Same rule as the askpass
/// helper: the saved password is never offered to anything else.
pub fn is_password_prompt(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    let lower = lower.trim();
    let asks_for_password =
        lower.contains("password:") || (lower.contains("password for ") && lower.ends_with(':'));
    asks_for_password
        && !lower.contains("passphrase")
        && !lower.contains("one-time")
        && !lower.contains("verification")
}

/// What an `OSC 133` mark said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Osc133Mark {
    PromptStart,
    PromptEnd,
    OutputStart,
    CommandDone(Option<i32>),
}

/// Finds the shell-integration marks (`OSC 133`) a remote shell may already
/// emit in the raw PTY bytes: `A` prompt start, `B` prompt end, `C` command
/// output start, `D[;exit]` command done. Either BEL or ST ends a mark, and a
/// mark may be split across chunks. Unlike Maine's own marker these carry no
/// nonce, so a program's output could fake one; that can only end a `run`
/// early, never reach a host or a secret.
#[derive(Debug, Clone, Default)]
pub struct Osc133Detector {
    matched: usize,
    body: Vec<u8>,
    in_body: bool,
    saw_escape: bool,
}

const OSC133_PREFIX: [u8; 6] = [0x1B, 0x5D, 0x31, 0x33, 0x33, 0x3B]; // ESC ] 1 3 3 ;
const OSC133_BODY_LIMIT: usize = 32;

impl Osc133Detector {
    pub fn new() -> Self {
        Self::default()
    }

    /// The marks found in `bytes`, each with the index just past its terminator.
    pub fn scan(&mut self, bytes: &[u8]) -> Vec<(Osc133Mark, usize)> {
        let mut found = Vec::new();
        for (index, &byte) in bytes.iter().enumerate() {
            if !self.in_body {
                if byte == OSC133_PREFIX[self.matched] {
                    self.matched += 1;
                    if self.matched == OSC133_PREFIX.len() {
                        self.in_body = true;
                        self.matched = 0;
                        self.body.clear();
                        self.saw_escape = false;
                    }
                } else {
                    self.matched = usize::from(byte == OSC133_PREFIX[0]);
                }
                continue;
            }
            if self.saw_escape {
                if byte == 0x5C {
                    if let Some(mark) = Self::parse(&self.body) {
                        found.push((mark, index + 1));
                    }
                }
                self.reset(byte == 0x1B);
            } else if byte == 0x07 {
                if let Some(mark) = Self::parse(&self.body) {
                    found.push((mark, index + 1));
                }
                self.reset(false);
            } else if byte == 0x1B {
                self.saw_escape = true;
            } else if self.body.len() >= OSC133_BODY_LIMIT {
                self.reset(false);
            } else {
                self.body.push(byte);
            }
        }
        found
    }

    fn reset(&mut self, retrying_escape: bool) {
        self.in_body = false;
        self.saw_escape = false;
        self.body.clear();
        self.matched = usize::from(retrying_escape);
    }

    fn parse(body: &[u8]) -> Option<Osc133Mark> {
        let text = String::from_utf8_lossy(body);
        let mut parts = text.split(';');
        match parts.next()? {
            "A" => Some(Osc133Mark::PromptStart),
            "B" => Some(Osc133Mark::PromptEnd),
            "C" => Some(Osc133Mark::OutputStart),
            "D" => Some(Osc133Mark::CommandDone(
                parts.next().and_then(|code| code.parse::<i32>().ok()),
            )),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // MARK: - CommandCompletionTests

    #[test]
    fn wrapper_text_is_exact() {
        let wrapper = CommandWrapper::with_nonce("ls -la", "ab12".into());
        assert_eq!(
            wrapper.typed_text,
            "{ ls -la\n}; printf '\\033]7770;ab12;%d\\007' \"$?\"\n"
        );
        assert_eq!(wrapper.nonce, "ab12");
    }

    #[test]
    fn marker_is_found_across_chunk_boundaries() {
        let marker = b"out\x1b]7770;ab12;3\x07more";
        for index in 0..=marker.len() {
            let mut detector = CompletionDetector::new("ab12");
            let first = detector.scan(&marker[..index]);
            let second = detector.scan(&marker[index..]);
            let found: Vec<i32> = [first, second].into_iter().flatten().collect();
            assert_eq!(found, vec![3], "split at {index}");
            assert_eq!(detector.scan(marker), None, "latched after the first match");
        }
    }

    #[test]
    fn echoed_wrapper_and_wrong_nonces_never_match() {
        let mut detector = CompletionDetector::new("ab12");
        assert_eq!(
            detector.scan(
                CommandWrapper::with_nonce("true", "ab12".into())
                    .typed_text
                    .as_bytes()
            ),
            None
        );
        assert_eq!(detector.scan(b"\x1b]7770;ffff;0\x07"), None);
        assert_eq!(detector.scan(b"\x1b]7770;ab12;x\x07"), None);
        assert_eq!(detector.scan(b"\x1b]7770;ab12;127\x07"), Some(127));
    }

    #[test]
    fn nonce_is_random_hex() {
        let (first, second) = (CommandWrapper::make_nonce(), CommandWrapper::make_nonce());
        assert_eq!(first.len(), 32);
        assert!(first.chars().all(|c| "0123456789abcdef".contains(c)));
        assert_ne!(first, second);
    }

    #[test]
    fn password_prompts_include_sudo() {
        for prompt in [
            "[sudo] password for alice: ",
            "Password:",
            "Password for alice@web: ",
        ] {
            assert!(is_password_prompt(prompt), "{prompt}");
        }
        for prompt in [
            "Enter passphrase for key '/x':",
            "One-time password:",
            "Verification code:",
        ] {
            assert!(!is_password_prompt(prompt), "{prompt}");
        }
    }

    #[test]
    fn shell_prompt_heuristic() {
        for line in ["alice@web:~$ ", "root@web:/# ", "web% ", "fish> "] {
            assert!(looks_like_shell_prompt(line), "{line}");
        }
        for line in ["Downloading 40%", "", "done"] {
            assert!(!looks_like_shell_prompt(line), "{line}");
        }
    }

    // Extra coverage for the Rust port.

    #[test]
    fn marker_with_an_overlong_exit_code_is_ignored() {
        let mut detector = CompletionDetector::new("ab12");
        assert_eq!(detector.scan(b"\x1b]7770;ab12;99999999999\x07"), None);
        assert_eq!(
            detector.scan(b"\x1b]7770;ab12;2147483647\x07"),
            Some(i32::MAX)
        );
    }

    #[test]
    fn scan_for_end_reports_where_the_marker_ends() {
        let mut detector = CompletionDetector::new("ab12");
        let bytes = b"hi\n\x1b]7770;ab12;0\x07$ ";
        let (code, end) = detector.scan_for_end(bytes).unwrap();
        assert_eq!(code, 0);
        assert_eq!(&bytes[end..], b"$ ");
    }

    // MARK: - OSC133Tests

    fn marks(text: &str, split: Option<usize>) -> Vec<Osc133Mark> {
        let bytes = text.as_bytes();
        let mut detector = Osc133Detector::new();
        let cut = split.unwrap_or(bytes.len());
        let mut found = detector.scan(&bytes[..cut]);
        found.extend(detector.scan(&bytes[cut..]));
        found.into_iter().map(|(mark, _)| mark).collect()
    }

    #[test]
    fn recognises_prompt_output_and_done_marks() {
        assert_eq!(
            marks("\x1b]133;A\x07$ \x1b]133;B\x07", None),
            vec![Osc133Mark::PromptStart, Osc133Mark::PromptEnd]
        );
        assert_eq!(
            marks("\x1b]133;C\x07hi\n\x1b]133;D;3\x07", None),
            vec![Osc133Mark::OutputStart, Osc133Mark::CommandDone(Some(3))]
        );
        assert_eq!(
            marks("\x1b]133;D\x07", None),
            vec![Osc133Mark::CommandDone(None)]
        );
        assert_eq!(
            marks("\x1b]133;D;0;aid=1\x07", None),
            vec![Osc133Mark::CommandDone(Some(0))]
        );
    }

    #[test]
    fn string_terminator_works_too() {
        assert_eq!(
            marks("\x1b]133;D;7\x1b\\", None),
            vec![Osc133Mark::CommandDone(Some(7))]
        );
    }

    #[test]
    fn marks_survive_any_chunk_split() {
        let text = "x\x1b]133;C\x07out\x1b]133;D;12\x1b\\y";
        for index in 0..=text.len() {
            assert_eq!(
                marks(text, Some(index)),
                vec![Osc133Mark::OutputStart, Osc133Mark::CommandDone(Some(12))],
                "split at {index}"
            );
        }
    }

    #[test]
    fn ignores_other_sequences_and_garbage() {
        assert!(marks("\x1b]7770;ab;0\x07\x1b]133\x07\x1b]1337;File=x\x07", None).is_empty());
        assert_eq!(
            marks("\x1b]133;D;notanumber\x07", None),
            vec![Osc133Mark::CommandDone(None)]
        );
        assert!(marks(&format!("\x1b]133;{}\x07", "9".repeat(500)), None).is_empty());
        assert!(marks("\x1b]133;D;1\x1b[0m", None).is_empty());
        assert_eq!(
            marks("\x1b]133;D;4\x07", None),
            vec![Osc133Mark::CommandDone(Some(4))]
        );
    }

    #[test]
    fn reports_where_each_mark_ends() {
        let mut detector = Osc133Detector::new();
        let bytes = b"ab\x1b]133;D;1\x07cd";
        let found = detector.scan(bytes);
        assert_eq!(found.len(), 1);
        assert_eq!(&bytes[found[0].1..], b"cd");
    }
}
