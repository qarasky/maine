//! OpenSSH asks for credentials by running `SSH_ASKPASS` again. That helper
//! (`maine --askpass <prompt>`) never holds a key: it asks the running app over
//! a 0600 Unix socket, and the app answers from the unlocked vault or with a
//! dialog. Only the host id travels in the environment.

use maine_core::vault::{Unlocked, VaultFile};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

pub const ENV_HOST_ID: &str = "MAINE_SSH_HOST_ID";
pub const ENV_SOCKET: &str = "MAINE_ASKPASS_SOCKET";

/// `Password:`, `user@host's password:`, `[sudo] password for alice:`,
/// `Password for alice@web:`. Never passphrases, one-time codes or MFA.
pub fn is_password_prompt(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    let lower = lower.trim();
    let asks =
        lower.contains("password:") || (lower.contains("password for ") && lower.ends_with(':'));
    asks && !lower.contains("passphrase")
        && !lower.contains("one-time")
        && !lower.contains("verification")
}

/// What the user is asked in a dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptRequest {
    pub host_name: String,
    pub prompt: String,
    /// Host-key trust and similar: a yes/no decision rather than text.
    pub confirmation: bool,
}

/// Implemented by the GUI. Blocks until the user answers; `None` = cancelled.
pub trait AskpassUi: Send + Sync {
    fn ask(&self, request: PromptRequest) -> Option<String>;
}

#[derive(Serialize, Deserialize)]
struct Request {
    #[serde(default)]
    host: String,
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    confirm: bool,
    /// `"show"`: a second `maine` launch asking the running one to raise its window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    action: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Response {
    answer: Option<String>,
}

/// Decides every askpass request. Holds the vault key only while unlocked.
pub struct Broker {
    vault: Mutex<Option<(VaultFile, Unlocked)>>,
    ui: Mutex<Option<Arc<dyn AskpassUi>>>,
    show_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl Broker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            vault: Mutex::new(None),
            ui: Mutex::new(None),
            show_hook: Mutex::new(None),
        })
    }

    pub fn set_ui(&self, ui: Arc<dyn AskpassUi>) {
        *self.ui.lock().unwrap() = Some(ui);
    }

    /// Called when a second launch asks this instance to show its window.
    pub fn set_show_hook(&self, hook: impl Fn() + Send + Sync + 'static) {
        *self.show_hook.lock().unwrap() = Some(Arc::new(hook));
    }

    fn show(&self) {
        let hook = self.show_hook.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook();
        }
    }

    pub fn set_vault(&self, vault: Option<(VaultFile, Unlocked)>) {
        *self.vault.lock().unwrap() = vault;
    }

    pub fn is_unlocked(&self) -> bool {
        self.vault.lock().unwrap().is_some()
    }

    /// The saved password for a host, if the vault is unlocked and has one.
    pub fn password(&self, host: Uuid) -> Option<String> {
        let guard = self.vault.lock().unwrap();
        let (file, unlocked) = guard.as_ref()?;
        file.contents(unlocked)
            .ok()?
            .passwords
            .get(&host)
            .cloned()
            .filter(|p| !p.is_empty())
    }

    fn host_name(&self, host: Uuid) -> String {
        let guard = self.vault.lock().unwrap();
        guard
            .as_ref()
            .and_then(|(file, unlocked)| file.contents(unlocked).ok())
            .and_then(|c| c.hosts.into_iter().find(|h| h.id == host))
            .map(|h| h.display_name())
            .unwrap_or_default()
    }

    /// The answer for one prompt; `None` makes ssh's askpass call fail.
    /// A locked vault fails closed: nothing is typed, no dialog is shown.
    pub fn answer(&self, host: Uuid, prompt: &str, confirm_env: bool) -> Option<String> {
        if !self.is_unlocked() {
            return None;
        }
        if is_password_prompt(prompt) {
            if let Some(password) = self.password(host) {
                return Some(password);
            }
        }
        // Never reuse a login password for key passphrases, MFA or host trust.
        let ui = self.ui.lock().unwrap().clone()?;
        let confirmation = confirm_env || prompt.contains("yes/no");
        ui.ask(PromptRequest {
            host_name: self.host_name(host),
            prompt: prompt.to_string(),
            confirmation,
        })
    }
}

/// The askpass listener. Dropping it stops accepting and removes the socket.
pub struct Server {
    path: PathBuf,
}

impl Server {
    pub fn start(path: &Path, broker: Arc<Broker>) -> std::io::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        let me = rustix::process::getuid();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let broker = broker.clone();
                std::thread::spawn(move || {
                    // Same user only, even though the socket is 0600.
                    if rustix::net::sockopt::socket_peercred(&stream)
                        .map(|c| c.uid != me)
                        .unwrap_or(true)
                    {
                        return;
                    }
                    handle(stream, &broker);
                });
            }
        });
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn handle(stream: UnixStream, broker: &Broker) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut line = String::new();
    // Requests are tiny; cap the read so a hostile peer cannot grow memory.
    if BufReader::new((&stream).take(64 * 1024))
        .read_line(&mut line)
        .is_err()
    {
        return;
    }
    let request = serde_json::from_str::<Request>(&line).ok();
    let answer = match request {
        Some(r) if r.action.as_deref() == Some("show") => {
            broker.show();
            Some(String::new())
        }
        Some(r) => Uuid::parse_str(&r.host)
            .ok()
            .and_then(|host| broker.answer(host, &r.prompt, r.confirm)),
        None => None,
    };
    let mut out = stream;
    let _ = out.set_write_timeout(Some(Duration::from_secs(10)));
    let _ = writeln!(
        out,
        "{}",
        serde_json::to_string(&Response { answer }).unwrap_or_default()
    );
}

/// A second `maine` launch: asks the running instance (if any) to show its
/// window. True when one answered, in which case this process should exit.
pub fn ask_running_instance_to_show(socket: &Path) -> bool {
    let Ok(mut stream) = UnixStream::connect(socket) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let request = Request {
        host: String::new(),
        prompt: String::new(),
        confirm: false,
        action: Some("show".into()),
    };
    if writeln!(
        stream,
        "{}",
        serde_json::to_string(&request).unwrap_or_default()
    )
    .is_err()
    {
        return false;
    }
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).is_ok()
        && serde_json::from_str::<Response>(&line).is_ok_and(|r| r.answer.is_some())
}

/// The helper side, run as `maine --askpass <prompt>`. Returns the process exit
/// code and what to print to stdout (an answer, or nothing).
pub fn run_helper(args: &[String], env: impl Fn(&str) -> Option<String>) -> (i32, String) {
    let (Some(prompt), Some(host)) = (args.get(1), env(ENV_HOST_ID)) else {
        return (1, String::new());
    };
    let Some(socket) = env(ENV_SOCKET) else {
        return (1, String::new());
    };
    let confirm = env("SSH_ASKPASS_PROMPT").as_deref() == Some("confirm");
    let Ok(mut stream) = UnixStream::connect(&socket) else {
        return (1, String::new());
    };
    // A dialog may stay open for a while; wait for the user.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(600)));
    let request = serde_json::to_string(&Request {
        host,
        prompt: prompt.clone(),
        confirm,
        action: None,
    })
    .unwrap_or_default();
    if writeln!(stream, "{request}").is_err() {
        return (1, String::new());
    }
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return (1, String::new());
    }
    match serde_json::from_str::<Response>(&line) {
        Ok(Response {
            answer: Some(answer),
        }) => (0, format!("{answer}\n")),
        _ => (1, String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maine_core::model::Host;

    #[test]
    fn password_prompts_are_recognised_and_other_prompts_are_not() {
        for yes in [
            "Password:",
            "alice@web's password:",
            "[sudo] password for alice:",
            "Password for alice@web:",
            "  PASSWORD:  ",
        ] {
            assert!(is_password_prompt(yes), "{yes}");
        }
        for no in [
            "Enter passphrase for key '/home/a/.ssh/id_ed25519':",
            "Verification code:",
            "One-time password (OATH) for alice:",
            "Are you sure you want to continue connecting (yes/no/[fingerprint])?",
            "",
        ] {
            assert!(!is_password_prompt(no), "{no}");
        }
    }

    struct Ui(Mutex<Vec<PromptRequest>>, Option<String>);
    impl AskpassUi for Ui {
        fn ask(&self, request: PromptRequest) -> Option<String> {
            self.0.lock().unwrap().push(request);
            self.1.clone()
        }
    }

    fn unlocked_broker(dir: &Path) -> (Arc<Broker>, Uuid, Arc<Ui>) {
        let file = VaultFile::new(dir.join("passwords.maine"));
        let unlocked = file.create("correct horse battery").unwrap();
        let host = Host::new("Web", "example.com");
        let id = host.id;
        file.update(&unlocked, |c| {
            c.hosts.push(host);
            c.passwords.insert(id, "s3cret".into());
        })
        .unwrap();
        let broker = Broker::new();
        broker.set_vault(Some((file, unlocked)));
        let ui = Arc::new(Ui(Mutex::new(vec![]), Some("typed".into())));
        broker.set_ui(ui.clone());
        (broker, id, ui)
    }

    #[test]
    fn saved_password_answers_password_prompts_only() {
        let d = tempfile::tempdir().unwrap();
        let (broker, id, ui) = unlocked_broker(d.path());
        assert_eq!(
            broker.answer(id, "web's password:", false).as_deref(),
            Some("s3cret")
        );
        assert!(ui.0.lock().unwrap().is_empty());
        // A passphrase never gets the login password; the user is asked.
        assert_eq!(
            broker
                .answer(id, "Enter passphrase for key 'k':", false)
                .as_deref(),
            Some("typed")
        );
        let asked = ui.0.lock().unwrap().clone();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].host_name, "Web");
        assert!(!asked[0].confirmation);
    }

    #[test]
    fn host_trust_prompts_are_confirmations() {
        let d = tempfile::tempdir().unwrap();
        let (broker, id, ui) = unlocked_broker(d.path());
        let _ = broker.answer(
            id,
            "Are you sure you want to continue connecting (yes/no)?",
            false,
        );
        assert!(ui.0.lock().unwrap()[0].confirmation);
    }

    #[test]
    fn locked_vault_fails_closed_without_a_dialog() {
        let d = tempfile::tempdir().unwrap();
        let (broker, id, ui) = unlocked_broker(d.path());
        broker.set_vault(None);
        assert_eq!(broker.answer(id, "Password:", false), None);
        assert_eq!(broker.answer(id, "Verification code:", false), None);
        assert!(ui.0.lock().unwrap().is_empty());
    }

    #[test]
    fn unknown_host_without_a_saved_password_asks_the_user() {
        let d = tempfile::tempdir().unwrap();
        let (broker, _id, ui) = unlocked_broker(d.path());
        assert_eq!(
            broker.answer(Uuid::new_v4(), "Password:", false).as_deref(),
            Some("typed")
        );
        assert_eq!(ui.0.lock().unwrap().len(), 1);
    }

    fn env_of(host: Uuid, socket: &Path) -> impl Fn(&str) -> Option<String> {
        let (host, socket) = (host.to_string(), socket.to_string_lossy().to_string());
        move |k| match k {
            ENV_HOST_ID => Some(host.clone()),
            ENV_SOCKET => Some(socket.clone()),
            _ => None,
        }
    }

    #[test]
    fn helper_round_trips_over_the_socket_with_0600_permissions() {
        let d = tempfile::tempdir().unwrap();
        let (broker, id, _ui) = unlocked_broker(d.path());
        let sock = d.path().join("run").join("askpass.sock");
        let server = Server::start(&sock, broker.clone()).unwrap();
        assert_eq!(
            std::fs::metadata(server.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(sock.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        let args = vec!["maine".to_string(), "alice@web's password:".to_string()];
        assert_eq!(
            run_helper(&args, env_of(id, &sock)),
            (0, "s3cret\n".to_string())
        );

        broker.set_vault(None);
        assert_eq!(run_helper(&args, env_of(id, &sock)), (1, String::new()));
    }

    #[test]
    fn helper_fails_when_nothing_is_listening_or_args_are_missing() {
        let d = tempfile::tempdir().unwrap();
        let sock = d.path().join("nope.sock");
        let args = vec!["maine".to_string(), "Password:".to_string()];
        assert_eq!(run_helper(&args, env_of(Uuid::new_v4(), &sock)).0, 1);
        assert_eq!(
            run_helper(&["maine".to_string()], env_of(Uuid::new_v4(), &sock)).0,
            1
        );
        assert_eq!(run_helper(&args, |_| None).0, 1);
    }

    #[test]
    fn a_second_launch_asks_the_running_instance_to_show_its_window() {
        let d = tempfile::tempdir().unwrap();
        let (broker, _, _) = unlocked_broker(d.path());
        let shown = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = shown.clone();
        broker.set_show_hook(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        let sock = d.path().join("run").join("a.sock");
        assert!(
            !ask_running_instance_to_show(&sock),
            "nothing listening yet"
        );
        let _server = Server::start(&sock, broker).unwrap();
        assert!(ask_running_instance_to_show(&sock));
        assert_eq!(shown.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn socket_is_removed_when_the_server_is_dropped() {
        let d = tempfile::tempdir().unwrap();
        let (broker, _, _) = unlocked_broker(d.path());
        let sock = d.path().join("run").join("a.sock");
        drop(Server::start(&sock, broker).unwrap());
        assert!(!sock.exists());
    }
}
