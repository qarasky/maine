//! QObjects exposed to QML. `AppController` is a thin shell over
//! `maine_engine::Engine`: every invokable locks the engine, does one thing and
//! returns an error message ("" = success) for QML to show.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, sidebar_json, cxx_name = "sidebarJson")]
        #[qproperty(QString, askpass_json, cxx_name = "askpassJson")]
        #[qproperty(i32, state)]
        #[qproperty(QString, version)]
        #[qproperty(i32, show_request, cxx_name = "showRequest")]
        #[qproperty(i32, quit_request, cxx_name = "quitRequest")]
        #[qproperty(bool, tray_active, cxx_name = "trayActive")]
        #[qproperty(QString, sync_json, cxx_name = "syncJson")]
        #[qproperty(QString, alert_json, cxx_name = "alertJson")]
        #[qproperty(QString, op_json, cxx_name = "opJson")]
        #[qproperty(QString, agent_json, cxx_name = "agentJson")]
        type AppController = super::AppControllerRust;

        /// Call once from QML after creation: lets background threads reach the UI thread.
        #[qinvokable]
        fn start(self: Pin<&mut AppController>);
        /// Recomputes the sidebar JSON; cheap when nothing changed.
        #[qinvokable]
        fn refresh(self: Pin<&mut AppController>);
        /// Periodic work (session checkpoint). Call about every 2 seconds.
        #[qinvokable]
        fn tick(self: Pin<&mut AppController>);

        #[qinvokable]
        #[cxx_name = "createVault"]
        fn create_vault(self: Pin<&mut AppController>, password: &QString) -> QString;
        #[qinvokable]
        fn unlock(self: Pin<&mut AppController>, password: &QString) -> QString;
        #[qinvokable]
        fn lock(self: Pin<&mut AppController>);
        /// Stops processes, keeps session records. The caller then quits Qt.
        #[qinvokable]
        fn quit(self: Pin<&mut AppController>);

        #[qinvokable]
        #[cxx_name = "openHost"]
        fn open_host(self: Pin<&mut AppController>, host_id: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "newSession"]
        fn new_session(self: Pin<&mut AppController>, host_id: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "selectSession"]
        fn select_session(self: Pin<&mut AppController>, session_id: &QString);
        #[qinvokable]
        #[cxx_name = "closeSession"]
        fn close_session(self: Pin<&mut AppController>, session_id: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "reconnectSession"]
        fn reconnect_session(self: Pin<&mut AppController>, session_id: &QString);
        #[qinvokable]
        #[cxx_name = "disconnectSession"]
        fn disconnect_session(self: Pin<&mut AppController>, session_id: &QString);
        #[qinvokable]
        #[cxx_name = "renameSession"]
        fn rename_session(
            self: Pin<&mut AppController>,
            session_id: &QString,
            name: &QString,
        ) -> QString;

        #[qinvokable]
        #[cxx_name = "hostForEditor"]
        fn host_for_editor(self: Pin<&mut AppController>, host_id: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "validateHost"]
        fn validate_host(self: Pin<&mut AppController>, host_json: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "saveHost"]
        fn save_host(self: Pin<&mut AppController>, host_json: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "deleteHost"]
        fn delete_host(self: Pin<&mut AppController>, host_id: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "duplicateHost"]
        fn duplicate_host(self: Pin<&mut AppController>, host_id: &QString) -> QString;
        #[qinvokable]
        fn credentials(self: Pin<&mut AppController>, host_id: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "addGroup"]
        fn add_group(self: Pin<&mut AppController>) -> QString;
        #[qinvokable]
        #[cxx_name = "renameGroup"]
        fn rename_group(
            self: Pin<&mut AppController>,
            group_id: &QString,
            name: &QString,
        ) -> QString;
        #[qinvokable]
        #[cxx_name = "deleteGroup"]
        fn delete_group(self: Pin<&mut AppController>, group_id: &QString);
        #[qinvokable]
        #[cxx_name = "moveGroup"]
        fn move_group(self: Pin<&mut AppController>, group_id: &QString, before_id: &QString);
        #[qinvokable]
        #[cxx_name = "moveHost"]
        fn move_host(
            self: Pin<&mut AppController>,
            host_id: &QString,
            group_id: &QString,
            before_id: &QString,
        );
        #[qinvokable]
        #[cxx_name = "setCollapsed"]
        fn set_collapsed(self: Pin<&mut AppController>, group_id: &QString, collapsed: bool);

        #[qinvokable]
        #[cxx_name = "importCandidates"]
        fn import_candidates(self: Pin<&mut AppController>) -> QString;
        #[qinvokable]
        #[cxx_name = "importHosts"]
        fn import_hosts(self: Pin<&mut AppController>, hosts_json: &QString) -> QString;

        /// The window regained focus: pull if the server changed.
        #[qinvokable]
        #[cxx_name = "syncForeground"]
        fn sync_foreground(self: Pin<&mut AppController>);
        /// Blocking sync operations run on a worker thread; the result arrives in `opJson`.
        #[qinvokable]
        #[cxx_name = "syncCreate"]
        fn sync_create(
            self: Pin<&mut AppController>,
            server: &QString,
            name: &QString,
            secret: &QString,
        );
        #[qinvokable]
        #[cxx_name = "syncJoin"]
        fn sync_join(
            self: Pin<&mut AppController>,
            server: &QString,
            token: &QString,
            name: &QString,
            master: &QString,
        );
        #[qinvokable]
        #[cxx_name = "syncPushEmpty"]
        fn sync_push_empty(
            self: Pin<&mut AppController>,
            server: &QString,
            token: &QString,
            name: &QString,
        );
        #[qinvokable]
        #[cxx_name = "syncRotate"]
        fn sync_rotate(self: Pin<&mut AppController>, profile_id: &QString);
        #[qinvokable]
        #[cxx_name = "syncDisable"]
        fn sync_disable(self: Pin<&mut AppController>, profile_id: &QString);
        #[qinvokable]
        #[cxx_name = "syncDeleteRemote"]
        fn sync_delete_remote(self: Pin<&mut AppController>, profile_id: &QString);
        #[qinvokable]
        #[cxx_name = "syncActivate"]
        fn sync_activate(self: Pin<&mut AppController>, profile_id: &QString);
        #[qinvokable]
        #[cxx_name = "syncRename"]
        fn sync_rename(
            self: Pin<&mut AppController>,
            profile_id: &QString,
            name: &QString,
        ) -> QString;

        #[qinvokable]
        #[cxx_name = "agentSetEnabled"]
        fn agent_set_enabled(self: Pin<&mut AppController>, enabled: bool);
        #[qinvokable]
        #[cxx_name = "agentAddClient"]
        fn agent_add_client(self: Pin<&mut AppController>, name: &QString);
        #[qinvokable]
        #[cxx_name = "agentRevoke"]
        fn agent_revoke(self: Pin<&mut AppController>, client_id: &QString);
        /// `{"groups": [uuid], "hosts": [uuid]}`
        #[qinvokable]
        #[cxx_name = "agentSetAccess"]
        fn agent_set_access(self: Pin<&mut AppController>, access_json: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "agentHandOff"]
        fn agent_hand_off(
            self: Pin<&mut AppController>,
            session_id: &QString,
            client_id: &QString,
        ) -> QString;
        #[qinvokable]
        #[cxx_name = "agentStop"]
        fn agent_stop(self: Pin<&mut AppController>, session_id: &QString);
        #[qinvokable]
        #[cxx_name = "agentStopAll"]
        fn agent_stop_all(self: Pin<&mut AppController>);
        #[qinvokable]
        #[cxx_name = "agentPause"]
        fn agent_pause(self: Pin<&mut AppController>, session_id: &QString, paused: bool);
        #[qinvokable]
        #[cxx_name = "agentKeep"]
        fn agent_keep(self: Pin<&mut AppController>, session_id: &QString);
        #[qinvokable]
        #[cxx_name = "agentLog"]
        fn agent_log(self: Pin<&mut AppController>) -> QString;
        #[qinvokable]
        #[cxx_name = "agentClearLog"]
        fn agent_clear_log(self: Pin<&mut AppController>);

        #[qinvokable]
        #[cxx_name = "answerAskpass"]
        fn answer_askpass(
            self: Pin<&mut AppController>,
            request_id: i32,
            text: &QString,
            accepted: bool,
        );
    }

    impl cxx_qt::Threading for AppController {}
}

use crate::state::with_engine;
use core::pin::Pin;
use cxx_qt::{CxxQtThread, Threading};
use cxx_qt_lib::QString;
use maine_core::ssh::SshConfigFile;
use maine_engine::askpass::{AskpassUi, PromptRequest};
use maine_engine::{view, EngineState};
use serde_json::json;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use uuid::Uuid;

pub struct AppControllerRust {
    sidebar_json: QString,
    askpass_json: QString,
    state: i32,
    version: QString,
    show_request: i32,
    quit_request: i32,
    tray_active: bool,
    sync_json: QString,
    alert_json: QString,
    op_json: QString,
    agent_json: QString,
}

impl Default for AppControllerRust {
    fn default() -> Self {
        Self {
            sidebar_json: QString::from(""),
            askpass_json: QString::from(""),
            state: 0,
            version: QString::from(env!("CARGO_PKG_VERSION")),
            show_request: 0,
            quit_request: 0,
            tray_active: false,
            sync_json: QString::from(""),
            alert_json: QString::from(""),
            op_json: QString::from(""),
            agent_json: QString::from(""),
        }
    }
}

/// Dialog requests from askpass helper threads to the UI thread and back.
struct AskpassBridge {
    thread: Mutex<Option<CxxQtThread<qobject::AppController>>>,
    pending: Mutex<HashMap<i32, Sender<Option<String>>>>,
    next: Mutex<i32>,
}

fn bridge() -> &'static AskpassBridge {
    static BRIDGE: OnceLock<AskpassBridge> = OnceLock::new();
    BRIDGE.get_or_init(|| AskpassBridge {
        thread: Mutex::new(None),
        pending: Mutex::new(HashMap::new()),
        next: Mutex::new(0),
    })
}

struct Ui;

impl AskpassUi for Ui {
    fn ask(&self, request: PromptRequest) -> Option<String> {
        let b = bridge();
        let thread = b.thread.lock().unwrap().clone()?;
        let (tx, rx) = channel();
        let id = {
            let mut next = b.next.lock().unwrap();
            *next += 1;
            *next
        };
        b.pending.lock().unwrap().insert(id, tx);
        let payload = json!({
            "id": id, "host": request.host_name, "prompt": request.prompt, "confirmation": request.confirmation,
        })
        .to_string();
        if thread
            .queue(move |mut controller| {
                controller
                    .as_mut()
                    .set_askpass_json(QString::from(&payload))
            })
            .is_err()
        {
            b.pending.lock().unwrap().remove(&id);
            return None;
        }
        let answer = rx.recv_timeout(Duration::from_secs(600)).ok().flatten();
        b.pending.lock().unwrap().remove(&id);
        answer
    }
}

/// Installs the dialog bridge into the engine's askpass broker.
pub fn install_askpass_ui() {
    with_engine(|e| e.broker().set_ui(std::sync::Arc::new(Ui)));
}

static OP_SEQ: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);

/// Asks the UI thread to recompute its snapshot (called from sync and tray threads).
pub fn request_refresh() {
    if let Some(thread) = bridge().thread.lock().unwrap().clone() {
        let _ = thread.queue(|c| c.refresh());
    }
}

/// Runs a blocking operation off the UI thread and delivers `{seq, op, ok, error, data}` in `opJson`.
fn run_op(
    thread: CxxQtThread<qobject::AppController>,
    op: &'static str,
    work: impl FnOnce() -> Result<serde_json::Value, String> + Send + 'static,
) {
    std::thread::spawn(move || {
        let result = work();
        let seq = OP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let payload = match result {
            Ok(data) => json!({"seq": seq, "op": op, "ok": true, "error": "", "data": data}),
            Err(error) => json!({"seq": seq, "op": op, "ok": false, "error": error, "data": null}),
        }
        .to_string();
        let _ = thread.queue(move |mut c| {
            c.as_mut().set_op_json(QString::from(&payload));
            c.refresh();
        });
    });
}

/// Requests from the tray thread to the UI thread.
pub fn request_show() {
    if let Some(thread) = bridge().thread.lock().unwrap().clone() {
        let _ = thread.queue(|mut c| {
            let n = *c.as_ref().show_request() + 1;
            c.as_mut().set_show_request(n);
        });
    }
}

pub fn request_lock() {
    crate::syncing::locked();
    crate::agents::vault_did_lock();
    with_engine(|e| e.lock());
    if let Some(thread) = bridge().thread.lock().unwrap().clone() {
        let _ = thread.queue(|c| c.refresh());
    }
}

pub fn request_quit() {
    if let Some(thread) = bridge().thread.lock().unwrap().clone() {
        let _ = thread.queue(|mut c| {
            let n = *c.as_ref().quit_request() + 1;
            c.as_mut().set_quit_request(n);
        });
    }
}

fn s(q: &QString) -> String {
    q.to_string()
}

fn uuid(q: &QString) -> Option<Uuid> {
    Uuid::parse_str(s(q).trim()).ok()
}

fn q(text: impl AsRef<str>) -> QString {
    QString::from(text.as_ref())
}

fn err<E: ToString>(r: Result<(), E>) -> QString {
    match r {
        Ok(()) => q(""),
        Err(e) => q(e.to_string()),
    }
}

impl qobject::AppController {
    fn publish(mut self: Pin<&mut Self>) {
        let (json, state) = with_engine(|e| {
            let state = match e.state() {
                EngineState::NoVault => 0,
                EngineState::Locked => 1,
                EngineState::Unlocked => 2,
            };
            let mut snapshot = view::sidebar(e);
            crate::agents::decorate(&mut snapshot);
            (snapshot.to_string(), state)
        });
        if self.as_ref().sidebar_json().to_string() != json {
            self.as_mut().set_sidebar_json(QString::from(&json));
        }
        if *self.as_ref().state() != state {
            self.as_mut().set_state(state);
        }
        let agents = crate::agents::json().to_string();
        if self.as_ref().agent_json().to_string() != agents {
            self.as_mut().set_agent_json(QString::from(&agents));
        }
        let sync = crate::syncing::json().to_string();
        if self.as_ref().sync_json().to_string() != sync {
            self.as_mut().set_sync_json(QString::from(&sync));
        }
        if let Some((title, message)) = crate::syncing::take_alert() {
            let n = OP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.as_mut().set_alert_json(QString::from(
                &json!({"seq": n, "title": title, "message": message}).to_string(),
            ));
        }
    }

    pub fn start(mut self: Pin<&mut Self>) {
        *bridge().thread.lock().unwrap() = Some(self.qt_thread());
        install_askpass_ui();
        with_engine(|e| e.broker().set_show_hook(request_show));
        let active = crate::tray::start();
        self.as_mut().set_tray_active(active);
        self.as_mut().publish();
    }

    pub fn refresh(self: Pin<&mut Self>) {
        self.publish();
    }

    pub fn tick(self: Pin<&mut Self>) {
        with_engine(|e| e.tick());
        self.publish();
    }

    pub fn create_vault(mut self: Pin<&mut Self>, password: &QString) -> QString {
        let result = with_engine(|e| e.create_vault(&s(password)));
        if result.is_ok() {
            crate::syncing::unlocked();
            crate::agents::vault_did_unlock();
        }
        self.as_mut().publish();
        err(result)
    }

    pub fn unlock(mut self: Pin<&mut Self>, password: &QString) -> QString {
        let result = with_engine(|e| e.unlock(&s(password)));
        if result.is_ok() {
            crate::syncing::unlocked();
            crate::agents::vault_did_unlock();
        }
        self.as_mut().publish();
        err(result)
    }

    pub fn lock(self: Pin<&mut Self>) {
        // Best-effort push first: it only reads the vault file, not the key.
        crate::syncing::locked();
        crate::agents::vault_did_lock();
        with_engine(|e| e.lock());
        self.publish();
    }

    pub fn quit(self: Pin<&mut Self>) {
        if let Some(c) = crate::agents::coordinator() {
            c.stop_all();
        }
        crate::syncing::flush_before_quit();
        with_engine(|e| e.quit());
    }

    pub fn open_host(mut self: Pin<&mut Self>, host_id: &QString) -> QString {
        let result = with_engine(|e| {
            let id = uuid(host_id).ok_or("Unknown host.")?;
            let host = e.host(id).cloned().ok_or("That host no longer exists.")?;
            e.sessions_mut().open(&host);
            Ok::<(), &str>(())
        });
        self.as_mut().publish();
        err(result)
    }

    pub fn new_session(mut self: Pin<&mut Self>, host_id: &QString) -> QString {
        let result = with_engine(|e| {
            let id = uuid(host_id).ok_or("Unknown host.")?;
            let host = e.host(id).cloned().ok_or("That host no longer exists.")?;
            e.sessions_mut().new_session(&host);
            Ok::<(), &str>(())
        });
        self.as_mut().publish();
        err(result)
    }

    pub fn select_session(self: Pin<&mut Self>, session_id: &QString) {
        if let Some(id) = uuid(session_id) {
            with_engine(|e| e.sessions_mut().select(id));
        }
        self.publish();
    }

    pub fn close_session(mut self: Pin<&mut Self>, session_id: &QString) -> QString {
        let result = match uuid(session_id) {
            Some(id) => with_engine(|e| e.sessions_mut().close_session(id)),
            None => Ok(()),
        };
        self.as_mut().publish();
        err(result)
    }

    pub fn reconnect_session(self: Pin<&mut Self>, session_id: &QString) {
        if let Some(id) = uuid(session_id) {
            with_engine(|e| {
                let host = e.sessions().host_of(id).and_then(|h| e.host(h.id).cloned());
                e.sessions_mut().reconnect_session(id, host.as_ref());
            });
        }
        self.publish();
    }

    pub fn disconnect_session(self: Pin<&mut Self>, session_id: &QString) {
        if let Some(id) = uuid(session_id) {
            with_engine(|e| e.sessions_mut().disconnect(id));
        }
        self.publish();
    }

    pub fn rename_session(
        mut self: Pin<&mut Self>,
        session_id: &QString,
        name: &QString,
    ) -> QString {
        let result = match uuid(session_id) {
            Some(id) => with_engine(|e| e.sessions_mut().rename(id, &s(name))),
            None => Ok(()),
        };
        self.as_mut().publish();
        err(result)
    }

    pub fn host_for_editor(self: Pin<&mut Self>, host_id: &QString) -> QString {
        let value = with_engine(|e| match uuid(host_id) {
            Some(id) => view::editor_host(e, id),
            None => json!({
                "id": Uuid::new_v4().to_string(), "name": "", "hostname": "", "port": 22,
                "username": "", "groupId": null, "password": "", "isNew": true,
            }),
        });
        q(value.to_string())
    }

    pub fn validate_host(self: Pin<&mut Self>, host_json: &QString) -> QString {
        match view::parse_editor_host(&s(host_json)) {
            Ok((host, _)) => q(host.issues().join("\n")),
            Err(message) => q(message),
        }
    }

    pub fn save_host(mut self: Pin<&mut Self>, host_json: &QString) -> QString {
        let result = view::parse_editor_host(&s(host_json)).and_then(|(host, password)| {
            with_engine(|e| e.save_host(host, &password)).map_err(|e| e.0)
        });
        self.as_mut().publish();
        err(result)
    }

    pub fn delete_host(mut self: Pin<&mut Self>, host_id: &QString) -> QString {
        let result = match uuid(host_id) {
            Some(id) => with_engine(|e| e.delete_host(id)).map_err(|e| e.0),
            None => Ok(()),
        };
        self.as_mut().publish();
        err(result)
    }

    pub fn duplicate_host(mut self: Pin<&mut Self>, host_id: &QString) -> QString {
        let result = with_engine(|e| {
            let id = uuid(host_id).ok_or_else(|| "Unknown host.".to_string())?;
            let mut copy = e
                .host(id)
                .cloned()
                .ok_or_else(|| "That host no longer exists.".to_string())?;
            let password = e.saved_password(id).unwrap_or_default();
            copy.id = Uuid::new_v4();
            copy.name = format!("{} copy", copy.name);
            e.save_host(copy, &password).map_err(|e| e.0)
        });
        self.as_mut().publish();
        err(result)
    }

    pub fn credentials(self: Pin<&mut Self>, host_id: &QString) -> QString {
        let text = with_engine(|e| uuid(host_id).and_then(|id| e.credential_text(id)));
        q(text.unwrap_or_default())
    }

    pub fn add_group(mut self: Pin<&mut Self>) -> QString {
        let id = with_engine(|e| {
            let mut created = None;
            let _ = e.update_layout(|l| {
                created = Some(l.add_group("New Group").id);
                Ok(())
            });
            created
        });
        self.as_mut().publish();
        q(id.map(|g| g.to_string()).unwrap_or_default())
    }

    pub fn rename_group(mut self: Pin<&mut Self>, group_id: &QString, name: &QString) -> QString {
        let result = match uuid(group_id) {
            Some(id) => {
                with_engine(|e| e.update_layout(|l| l.rename_group(id, &s(name)))).map_err(|e| e.0)
            }
            None => Ok(()),
        };
        self.as_mut().publish();
        err(result)
    }

    pub fn delete_group(self: Pin<&mut Self>, group_id: &QString) {
        if let Some(id) = uuid(group_id) {
            let _ = with_engine(|e| {
                e.update_layout(|l| {
                    l.delete_group(id);
                    Ok(())
                })
            });
        }
        self.publish();
    }

    pub fn move_group(self: Pin<&mut Self>, group_id: &QString, before_id: &QString) {
        if let Some(id) = uuid(group_id) {
            let before = uuid(before_id);
            let _ = with_engine(|e| {
                e.update_layout(|l| {
                    l.move_group(id, before);
                    Ok(())
                })
            });
        }
        self.publish();
    }

    pub fn move_host(
        self: Pin<&mut Self>,
        host_id: &QString,
        group_id: &QString,
        before_id: &QString,
    ) {
        if let Some(id) = uuid(host_id) {
            let (group, before) = (uuid(group_id), uuid(before_id));
            let _ = with_engine(|e| {
                e.update_layout(|l| {
                    l.move_host(id, group, before);
                    Ok(())
                })
            });
        }
        self.publish();
    }

    pub fn set_collapsed(self: Pin<&mut Self>, group_id: &QString, collapsed: bool) {
        if let Some(id) = uuid(group_id) {
            let _ = with_engine(|e| e.set_collapsed(id, collapsed));
        }
        self.publish();
    }

    pub fn import_candidates(self: Pin<&mut Self>) -> QString {
        let hosts = with_engine(|e| {
            e.ssh_config_candidates(&SshConfigFile::load(&SshConfigFile::default_path()))
        });
        let list: Vec<_> = hosts
            .iter()
            .map(|h| json!({"name": h.name, "hostname": h.hostname, "port": h.port, "username": h.username}))
            .collect();
        q(serde_json::Value::Array(list).to_string())
    }

    pub fn import_hosts(mut self: Pin<&mut Self>, hosts_json: &QString) -> QString {
        let result = with_engine(|e| {
            let items: Vec<serde_json::Value> = serde_json::from_str(&s(hosts_json))
                .map_err(|_| "Nothing to import.".to_string())?;
            for item in items {
                let (host, _) = view::parse_editor_host(&item.to_string())?;
                e.save_host(host, "").map_err(|e| e.0)?;
            }
            Ok::<(), String>(())
        });
        self.as_mut().publish();
        err(result)
    }

    pub fn sync_foreground(self: Pin<&mut Self>) {
        crate::syncing::foreground();
    }

    pub fn sync_create(self: Pin<&mut Self>, server: &QString, name: &QString, secret: &QString) {
        let (server, name, secret) = (s(server), s(name), s(secret));
        run_op(self.qt_thread(), "create", move || {
            let handle = crate::syncing::handle().ok_or("Sync is not available.")?;
            let secret = (!secret.trim().is_empty()).then_some(secret.trim().to_string());
            let created = handle
                .create_profile(&server, &name, secret.as_deref())
                .map_err(|e| e.to_string())?;
            let shown = maine_sync::token::display_token(&created.token).unwrap_or(created.token);
            Ok(json!({"token": shown, "profile": created.profile.name}))
        });
    }

    pub fn sync_join(
        self: Pin<&mut Self>,
        server: &QString,
        token: &QString,
        name: &QString,
        master: &QString,
    ) {
        let (server, token, name, master) = (s(server), s(token), s(name), s(master));
        run_op(self.qt_thread(), "join", move || {
            let handle = crate::syncing::handle().ok_or("Sync is not available.")?;
            match handle
                .join_profile(&server, &token, &name, &master)
                .map_err(|e| e.to_string())?
            {
                maine_sync::JoinOutcome::Joined(profile) => {
                    // The server's vault is now the live file, still locked: open it with the password just verified.
                    with_engine(|e| e.unlock(&master)).map_err(|e| e.to_string())?;
                    handle.unlocked();
                    Ok(json!({"joined": true, "profile": profile.name}))
                }
                maine_sync::JoinOutcome::EmptySlot => {
                    Ok(json!({"joined": false, "emptySlot": true}))
                }
            }
        });
    }

    pub fn sync_push_empty(
        self: Pin<&mut Self>,
        server: &QString,
        token: &QString,
        name: &QString,
    ) {
        let (server, token, name) = (s(server), s(token), s(name));
        run_op(self.qt_thread(), "pushEmpty", move || {
            let handle = crate::syncing::handle().ok_or("Sync is not available.")?;
            let profile = handle
                .push_local_vault_into_empty_slot(&server, &token, &name)
                .map_err(|e| e.to_string())?;
            Ok(json!({"profile": profile.name}))
        });
    }

    pub fn sync_rotate(self: Pin<&mut Self>, profile_id: &QString) {
        let id = uuid(profile_id);
        run_op(self.qt_thread(), "rotate", move || {
            let handle = crate::syncing::handle().ok_or("Sync is not available.")?;
            let token = handle
                .rotate_token(id.ok_or("Unknown profile.")?)
                .map_err(|e| e.to_string())?;
            Ok(json!({"token": maine_sync::token::display_token(&token).unwrap_or(token)}))
        });
    }

    pub fn sync_disable(self: Pin<&mut Self>, profile_id: &QString) {
        let id = uuid(profile_id);
        run_op(self.qt_thread(), "disable", move || {
            let handle = crate::syncing::handle().ok_or("Sync is not available.")?;
            handle
                .disable(id.ok_or("Unknown profile.")?)
                .map_err(|e| e.to_string())?;
            Ok(json!({}))
        });
    }

    pub fn sync_delete_remote(self: Pin<&mut Self>, profile_id: &QString) {
        let id = uuid(profile_id);
        run_op(self.qt_thread(), "deleteRemote", move || {
            let handle = crate::syncing::handle().ok_or("Sync is not available.")?;
            handle
                .delete_remote_data(id.ok_or("Unknown profile.")?)
                .map_err(|e| e.to_string())?;
            Ok(json!({}))
        });
    }

    pub fn sync_activate(self: Pin<&mut Self>, profile_id: &QString) {
        let id = uuid(profile_id);
        run_op(self.qt_thread(), "activate", move || {
            let handle = crate::syncing::handle().ok_or("Sync is not available.")?;
            handle
                .activate(id.ok_or("Unknown profile.")?)
                .map_err(|e| e.to_string())?;
            Ok(json!({}))
        });
    }

    pub fn sync_rename(self: Pin<&mut Self>, profile_id: &QString, name: &QString) -> QString {
        let Some(id) = uuid(profile_id) else {
            return q("Unknown profile.");
        };
        let result = match crate::syncing::handle() {
            Some(h) => h
                .rename_profile(id, &s(name))
                .map(|_| ())
                .map_err(|e| e.to_string()),
            None => Err("Sync is not available.".to_string()),
        };
        self.publish();
        err(result)
    }

    pub fn agent_set_enabled(self: Pin<&mut Self>, enabled: bool) {
        crate::agents::set_enabled(enabled);
        self.publish();
    }

    pub fn agent_add_client(self: Pin<&mut Self>, name: &QString) {
        let name = s(name);
        run_op(self.qt_thread(), "agentPair", move || {
            let c = crate::agents::coordinator().ok_or("Agent access is not available.")?;
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let pairing = c
                .add_client(&name, &exe.to_string_lossy())
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "name": pairing.client.name,
                "token": pairing.token,
                "claudeCodeCommand": pairing.claude_code_command,
                "desktopJson": pairing.desktop_json,
            }))
        });
    }

    pub fn agent_revoke(self: Pin<&mut Self>, client_id: &QString) {
        if let (Some(id), Some(c)) = (uuid(client_id), crate::agents::coordinator()) {
            c.revoke(id);
        }
        self.publish();
    }

    pub fn agent_set_access(self: Pin<&mut Self>, access_json: &QString) -> QString {
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&s(access_json));
        let result = match (parsed, crate::agents::coordinator()) {
            (Ok(v), Some(c)) => {
                let ids = |key: &str| -> Vec<Uuid> {
                    v[key]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|x| x.as_str())
                        .filter_map(|x| Uuid::parse_str(x).ok())
                        .collect()
                };
                c.set_access(&maine_agents::AgentAccess::new(ids("groups"), ids("hosts")));
                Ok(())
            }
            _ => Err("The access rules could not be read.".to_string()),
        };
        self.publish();
        err(result)
    }

    pub fn agent_hand_off(
        self: Pin<&mut Self>,
        session_id: &QString,
        client_id: &QString,
    ) -> QString {
        let result = match (
            uuid(session_id),
            uuid(client_id),
            crate::agents::coordinator(),
        ) {
            (Some(session), Some(client), Some(c)) => {
                c.hand_to_agent(session, client).map_err(|e| e.to_string())
            }
            _ => Err("That session or agent no longer exists.".to_string()),
        };
        self.publish();
        err(result)
    }

    pub fn agent_stop(self: Pin<&mut Self>, session_id: &QString) {
        if let (Some(id), Some(c)) = (uuid(session_id), crate::agents::coordinator()) {
            c.stop(id);
        }
        self.publish();
    }

    pub fn agent_stop_all(self: Pin<&mut Self>) {
        if let Some(c) = crate::agents::coordinator() {
            c.stop_all();
        }
        self.publish();
    }

    pub fn agent_pause(self: Pin<&mut Self>, session_id: &QString, paused: bool) {
        if let Some(id) = uuid(session_id) {
            crate::agents::pause(id, paused);
        }
        self.publish();
    }

    pub fn agent_keep(self: Pin<&mut Self>, session_id: &QString) {
        if let Some(id) = uuid(session_id) {
            with_engine(|e| e.sessions_mut().keep_session(id));
        }
        self.publish();
    }

    pub fn agent_log(self: Pin<&mut Self>) -> QString {
        q(crate::agents::audit_json().to_string())
    }

    pub fn agent_clear_log(self: Pin<&mut Self>) {
        crate::agents::clear_audit();
    }

    pub fn answer_askpass(self: Pin<&mut Self>, request_id: i32, text: &QString, accepted: bool) {
        if let Some(tx) = bridge().pending.lock().unwrap().remove(&request_id) {
            let _ = tx.send(accepted.then(|| s(text)));
        }
    }
}
