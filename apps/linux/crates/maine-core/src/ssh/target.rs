use super::SshConfigFile;
use crate::model::Host;

/// The system OpenSSH client. Maine never ships its own.
pub const SSH_PATH: &str = "/usr/bin/ssh";

/// The exact `ssh` command Maine runs for a host. It passes as little as
/// possible so keys, agent, `ProxyJump` and `Match` keep working: `-p` only when
/// the port is not 22, `-l` only when a username is set. Never a password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshTarget {
    pub executable: String,
    pub arguments: Vec<String>,
}

/// The connection as OpenSSH resolves it after `~/.ssh/config`, best effort,
/// for display only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub hostname: String,
    pub username: String,
    pub port: i64,
    pub config_pattern: Option<String>,
}

impl Resolution {
    pub fn target(&self) -> String {
        format!("{}@{}:{}", self.username, self.hostname, self.port)
    }
}

impl SshTarget {
    pub fn make(host: &Host) -> Self {
        let mut arguments = Vec::new();
        if host.port != Host::DEFAULT_PORT {
            arguments.extend(["-p".to_string(), host.port.to_string()]);
        }
        if !host.username.is_empty() {
            arguments.extend(["-l".to_string(), host.username.clone()]);
        }
        arguments.push(host.hostname.clone());
        Self {
            executable: SSH_PATH.to_string(),
            arguments,
        }
    }

    pub fn is_available() -> bool {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(SSH_PATH)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    /// For display and for copying to a terminal.
    pub fn command_line(&self) -> String {
        std::iter::once(self.executable.as_str())
            .chain(self.arguments.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn resolved(&self, config: &SshConfigFile) -> Resolution {
        let Some(address) = self.arguments.last() else {
            return Resolution {
                hostname: String::new(),
                username: local_user(),
                port: Host::DEFAULT_PORT,
                config_pattern: None,
            };
        };
        let entry = config.entry(address);
        let username = self
            .value_of("-l")
            .or_else(|| entry.and_then(|e| e.user.clone()))
            .unwrap_or_else(local_user);
        let port = match self.value_of("-p") {
            Some(explicit) => explicit.parse().unwrap_or(Host::DEFAULT_PORT),
            None => entry.and_then(|e| e.port).unwrap_or(Host::DEFAULT_PORT),
        };
        Resolution {
            hostname: entry
                .and_then(|e| e.hostname.clone())
                .unwrap_or_else(|| address.clone()),
            username,
            port,
            config_pattern: entry.map(|e| e.pattern.clone()),
        }
    }

    fn value_of(&self, flag: &str) -> Option<String> {
        let index = self.arguments.iter().position(|a| a == flag)?;
        self.arguments.get(index + 1).cloned()
    }
}

fn local_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default()
}

/// The environment the child `ssh` runs with: the parent's, inherited whole
/// (so `SSH_AUTH_SOCK`, `PATH`, proxies and locale keep working), with only
/// `TERM`, `COLORTERM` and, when unset, `LANG` set.
pub fn terminal_environment() -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| k != "TERM" && k != "COLORTERM")
        .collect();
    env.push(("TERM".into(), "xterm-256color".into()));
    env.push(("COLORTERM".into(), "truecolor".into()));
    if !env.iter().any(|(k, v)| k == "LANG" && !v.is_empty()) {
        env.retain(|(k, _)| k != "LANG");
        env.push(("LANG".into(), "en_US.UTF-8".into()));
    }
    env
}

#[cfg(test)]
mod tests {
    use super::super::SshConfigEntry;
    use super::*;

    fn address() -> Host {
        Host::new("Web", "example.com")
    }

    #[test]
    fn command_uses_the_system_openssh_client() {
        assert_eq!(SSH_PATH, "/usr/bin/ssh");
    }

    #[test]
    fn default_host_gets_no_options() {
        assert_eq!(SshTarget::make(&address()).arguments, ["example.com"]);
    }

    #[test]
    fn non_default_port_is_passed() {
        let mut h = address();
        h.port = 2222;
        assert_eq!(SshTarget::make(&h).arguments, ["-p", "2222", "example.com"]);
    }

    #[test]
    fn username_is_passed_only_when_set() {
        let mut h = address();
        h.username = "deploy".into();
        assert_eq!(
            SshTarget::make(&h).arguments,
            ["-l", "deploy", "example.com"]
        );
    }

    #[test]
    fn both_options_in_a_conventional_order() {
        let mut h = address();
        h.port = 2222;
        h.username = "deploy".into();
        assert_eq!(
            SshTarget::make(&h).arguments,
            ["-p", "2222", "-l", "deploy", "example.com"]
        );
        assert_eq!(
            SshTarget::make(&h).command_line(),
            "/usr/bin/ssh -p 2222 -l deploy example.com"
        );
    }

    fn entry(pattern: &str) -> SshConfigEntry {
        SshConfigEntry {
            pattern: pattern.into(),
            hostname: None,
            user: None,
            port: None,
        }
    }

    #[test]
    fn resolution_fills_in_the_local_user_when_nothing_is_set() {
        let r = SshTarget::make(&address()).resolved(&SshConfigFile::default());
        assert_eq!(r.hostname, "example.com");
        assert_eq!(r.port, 22);
        assert_eq!(r.username, local_user());
        assert_eq!(r.config_pattern, None);
    }

    #[test]
    fn resolution_takes_values_from_a_ssh_config_stanza() {
        let config = SshConfigFile {
            entries: vec![SshConfigEntry {
                hostname: Some("10.1.2.3".into()),
                user: Some("svc".into()),
                port: Some(2222),
                ..entry("example.com")
            }],
        };
        let r = SshTarget::make(&address()).resolved(&config);
        assert_eq!(
            (r.hostname.as_str(), r.username.as_str(), r.port),
            ("10.1.2.3", "svc", 2222)
        );
        assert_eq!(r.config_pattern.as_deref(), Some("example.com"));
        assert_eq!(r.target(), "svc@10.1.2.3:2222");
    }

    #[test]
    fn explicit_port_and_username_win_over_the_config() {
        let config = SshConfigFile {
            entries: vec![SshConfigEntry {
                port: Some(2222),
                user: Some("svc".into()),
                ..entry("example.com")
            }],
        };
        let mut h = address();
        h.port = 2200;
        h.username = "deploy".into();
        let r = SshTarget::make(&h).resolved(&config);
        assert_eq!((r.port, r.username.as_str()), (2200, "deploy"));
    }

    #[test]
    fn default_port_lets_the_config_decide() {
        let config = SshConfigFile {
            entries: vec![SshConfigEntry {
                port: Some(2222),
                ..entry("example.com")
            }],
        };
        assert_eq!(SshTarget::make(&address()).resolved(&config).port, 2222);
    }

    #[test]
    fn terminal_variables_are_set_and_parent_env_is_inherited() {
        let env: std::collections::HashMap<_, _> = terminal_environment().into_iter().collect();
        assert_eq!(env["TERM"], "xterm-256color");
        assert_eq!(env["COLORTERM"], "truecolor");
        assert!(env.contains_key("LANG") || env.contains_key("LC_ALL"));
        for key in ["PATH", "HOME", "SSH_AUTH_SOCK"] {
            if let Ok(expected) = std::env::var(key) {
                assert_eq!(env[key], expected);
            }
        }
    }
}
