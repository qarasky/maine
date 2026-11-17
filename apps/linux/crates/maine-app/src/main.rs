mod agents;
mod bridge;
mod ffi;
mod secrets;
mod state;
mod syncing;
mod tray;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QString, QUrl};
use maine_engine::{askpass, Engine, Paths};

extern "C" {
    fn maine_register_qml_types();
}

fn main() {
    // `maine --mcp`: the stdio bridge an MCP client starts; it talks to the running app over a socket.
    if std::env::args().nth(1).as_deref() == Some("--mcp") {
        std::process::exit(maine_agents::run_bridge_from_env());
    }

    // `ssh` runs us again as SSH_ASKPASS: ask the running app and print the answer.
    if std::env::var_os("MAINE_SSH_ASKPASS").is_some() {
        let args: Vec<String> = std::env::args().collect();
        let (code, output) = askpass::run_helper(&args, |k| std::env::var(k).ok());
        print!("{output}");
        std::process::exit(code);
    }

    if std::env::args().any(|a| a == "--debug") {
        maine_engine::diag::set_enabled(true);
    }
    let paths = match std::env::var_os("MAINE_DATA_DIR") {
        // Dev and tests: everything under one directory.
        Some(dir) => Paths::under(std::path::Path::new(&dir)),
        None => Paths::from_env(),
    };
    // Single instance: a second launch raises the running window and exits.
    if askpass::ask_running_instance_to_show(&paths.askpass_socket()) {
        return;
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| "maine".into());
    let config_dir = paths.config_dir.clone();
    let agent_paths = paths.clone();
    state::init(make_engine(paths, exe));
    state::with_engine(|e| e.set_write_hook(syncing::notify_local_write));
    agents::init(&agent_paths);
    if let Err(message) = syncing::init(config_dir) {
        eprintln!("maine: sync unavailable: {message}");
    }

    let mut app = QGuiApplication::new();
    if std::env::var_os("QT_QUICK_CONTROLS_STYLE").is_none() {
        cxx_qt_lib::QQuickStyle::set_style(&QString::from("org.kde.desktop"));
    }
    let mut engine = QQmlApplicationEngine::new();
    // SAFETY: plain C++ function registering the terminal item type.
    unsafe { maine_register_qml_types() };
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/app/maine/ui/qml/Main.qml"));
    }
    if let Some(app) = app.as_mut() {
        app.exec();
    }
    syncing::flush_before_quit();
    state::with_engine(|e| e.quit());
}

/// Debug builds can replace ssh with a shell script (`MAINE_DEBUG_SHELL`) so
/// the UI can be exercised without a server. Release builds always run ssh.
#[cfg(debug_assertions)]
fn make_engine(paths: Paths, exe: std::path::PathBuf) -> Engine {
    use maine_core::model::Host;
    use maine_engine::Launcher;
    use maine_term::{SessionConfig, TermSession};

    struct Shell(String);
    impl Launcher for Shell {
        fn launch(
            &self,
            _: &Host,
            cols: u16,
            rows: u16,
            history: Option<&str>,
        ) -> Result<TermSession, String> {
            let mut c = SessionConfig::new("/bin/sh", vec!["-c".into(), self.0.clone()]);
            // Same terminal environment the real ssh launcher uses (TERM=xterm-256color, ...).
            c.env = maine_core::ssh::terminal_environment();
            c.cols = cols;
            c.rows = rows;
            c.history = history.map(str::to_string);
            TermSession::spawn(c)
        }
    }
    match std::env::var("MAINE_DEBUG_SHELL") {
        Ok(script) => {
            let mut engine = Engine::with_launcher(
                paths,
                askpass::Broker::new(),
                std::sync::Arc::new(Shell(script)),
            );
            // Same single-instance / askpass socket as the real engine.
            engine.start_askpass_server();
            engine
        }
        Err(_) => Engine::new(paths, exe),
    }
}

#[cfg(not(debug_assertions))]
fn make_engine(paths: Paths, exe: std::path::PathBuf) -> Engine {
    Engine::new(paths, exe)
}
