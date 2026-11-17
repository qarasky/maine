use crate::model::Host;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// One `Host` pattern found in `~/.ssh/config`. Only the directives Maine needs
/// to build a host record are read; everything else is left to OpenSSH.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SshConfigEntry {
    pub pattern: String,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<i64>,
}

impl SshConfigEntry {
    /// True when the pattern names exactly one host (no wildcards).
    pub fn is_connectable(pattern: &str) -> bool {
        !pattern.is_empty()
            && !pattern.chars().any(char::is_whitespace)
            && !pattern.contains(['*', '?'])
    }

    /// A Maine host from this entry; `None` unless the pattern is connectable.
    pub fn to_host(&self) -> Option<Host> {
        if !Self::is_connectable(&self.pattern) {
            return None;
        }
        let mut host = Host::new(self.pattern.clone(), self.pattern.clone());
        host.port = self.port.unwrap_or(Host::DEFAULT_PORT);
        host.username = self.user.clone().unwrap_or_default();
        Some(host)
    }
}

/// A best-effort, read-only reader of the OpenSSH client configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SshConfigFile {
    pub entries: Vec<SshConfigEntry>,
}

impl SshConfigFile {
    pub fn default_path() -> PathBuf {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(".ssh")
            .join("config")
    }

    /// A missing or unreadable file is not an error: there is just nothing to import.
    pub fn load(path: &Path) -> Self {
        let mut entries = Vec::new();
        parse(path, &mut entries, &mut HashSet::new(), 0);
        Self { entries }
    }

    pub fn entry(&self, pattern: &str) -> Option<&SshConfigEntry> {
        self.entries.iter().find(|e| {
            e.pattern.eq_ignore_ascii_case(pattern)
                || e.pattern.to_lowercase() == pattern.to_lowercase()
        })
    }
}

fn parse(
    path: &Path,
    entries: &mut Vec<SshConfigEntry>,
    visited: &mut HashSet<PathBuf>,
    depth: usize,
) {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if depth >= 8 || !visited.insert(canonical) {
        return;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    let contents = String::from_utf8_lossy(&bytes);
    let mut active: Vec<String> = Vec::new();

    for line in contents.split('\n') {
        let tokens = tokenize(line);
        let Some(keyword) = tokens.first().map(|k| k.to_lowercase()) else {
            continue;
        };
        let values = &tokens[1..];
        match keyword.as_str() {
            "host" => {
                active.clear();
                for pattern in values.iter().filter(|p| !p.is_empty()) {
                    if !entries.iter().any(|e| &e.pattern == pattern) {
                        entries.push(SshConfigEntry {
                            pattern: pattern.clone(),
                            hostname: None,
                            user: None,
                            port: None,
                        });
                    }
                    active.push(pattern.clone());
                }
            }
            "hostname" => set_first(
                entries,
                &active,
                |e| &mut e.hostname,
                values.first().cloned(),
            ),
            "user" => set_first(entries, &active, |e| &mut e.user, values.first().cloned()),
            "port" => {
                if let Some(port) = values.first().and_then(|v| v.parse::<i64>().ok()) {
                    set_first(entries, &active, |e| &mut e.port, Some(port));
                }
            }
            "include" => {
                let base = path.parent().unwrap_or(Path::new("."));
                for argument in values {
                    for included in expand_include(argument, base) {
                        parse(&included, entries, visited, depth + 1);
                    }
                }
            }
            _ => {}
        }
    }
}

/// OpenSSH keeps the first value it obtains for a keyword, so a later block can
/// still supply a value an earlier one left out.
fn set_first<T>(
    entries: &mut [SshConfigEntry],
    active: &[String],
    field: impl Fn(&mut SshConfigEntry) -> &mut Option<T>,
    value: Option<T>,
) where
    T: Clone,
{
    let Some(value) = value else { return };
    for entry in entries.iter_mut().filter(|e| active.contains(&e.pattern)) {
        let slot = field(entry);
        if slot.is_none() {
            *slot = Some(value.clone());
        }
    }
}

/// Splits a line into tokens: whitespace and `=` separate, comments dropped, quotes stripped.
fn tokenize(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for c in strip_comment(line).chars() {
        if c == ' ' || c == '\t' || c == '=' || c == '\r' {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens.into_iter().map(|t| unquote(&t)).collect()
}

/// A `#` only opens a comment at the start of a line or after whitespace.
fn strip_comment(line: &str) -> &str {
    match line.char_indices().find(|&(i, c)| {
        c == '#'
            && (i == 0
                || line[..i]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace))
    }) {
        Some((i, _)) => &line[..i],
        None => line,
    }
}

fn unquote(token: &str) -> String {
    let b = token.as_bytes();
    if b.len() >= 2
        && ((b[0] == b'"' && b[b.len() - 1] == b'"') || (b[0] == b'\'' && b[b.len() - 1] == b'\''))
    {
        token[1..token.len() - 1].to_string()
    } else {
        token.to_string()
    }
}

/// Expands `~`, `{a,b}` braces and wildcards in an `Include` argument; relative
/// arguments resolve against the including file's directory.
fn expand_include(argument: &str, base: &Path) -> Vec<PathBuf> {
    let expanded = if let Some(rest) = argument.strip_prefix("~/") {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(rest)
    } else if argument == "~" {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
    } else {
        PathBuf::from(argument)
    };
    let pattern = if expanded.is_absolute() {
        expanded
    } else {
        base.join(expanded)
    };
    let mut out = Vec::new();
    for variant in expand_braces(&pattern.to_string_lossy()) {
        if let Ok(paths) = glob::glob(&variant) {
            out.extend(paths.flatten());
        }
    }
    out
}

fn expand_braces(pattern: &str) -> Vec<String> {
    let Some(open) = pattern.find('{') else {
        return vec![pattern.to_string()];
    };
    let mut depth = 0;
    let mut close = None;
    for (i, c) in pattern[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(close) = close else {
        return vec![pattern.to_string()];
    };
    let (head, inner, tail) = (
        &pattern[..open],
        &pattern[open + 1..close],
        &pattern[close + 1..],
    );
    let mut options = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (i, c) in inner.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                options.push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    options.push(&inner[start..]);
    options
        .into_iter()
        .flat_map(|o| expand_braces(&format!("{head}{o}{tail}")))
        .collect()
}
