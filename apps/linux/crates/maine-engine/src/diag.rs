//! Opt-in diagnostics (`maine --debug`). Metadata only: event names, session
//! and host UUIDs, counts. Never passwords, keystrokes, host names, user names,
//! custom session names or terminal contents (AGENTS.md invariant 8).

use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// One line on stderr: `maine: <event> key=value ...`. Values are expected to
/// be ids or numbers; callers must not pass user-entered text.
pub fn event(name: &str, fields: &[(&str, String)]) {
    if enabled() {
        eprintln!("{}", line(name, fields));
    }
}

fn line(name: &str, fields: &[(&str, String)]) -> String {
    let mut out = format!("maine: {name}");
    for (key, value) in fields {
        out.push_str(&format!(" {key}={value}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_event_plus_key_values() {
        assert_eq!(
            line(
                "session.open",
                &[("session", "abc".into()), ("count", "2".into())]
            ),
            "maine: session.open session=abc count=2"
        );
        assert_eq!(line("quit", &[]), "maine: quit");
    }

    #[test]
    fn disabled_by_default_and_toggleable() {
        set_enabled(false);
        assert!(!enabled());
        set_enabled(true);
        assert!(enabled());
        set_enabled(false);
    }
}
