//! The app side of the agent socket: a Unix stream socket that only this user
//! can open (directory 0700, socket 0600, peer uid checked with `SO_PEERCRED`).
//! Every connection must present a paired client's token in its first frame.
//!
//! One reader thread per connection reads frames; each tool call runs on its
//! own thread (calls such as `run` block for a long time and a bridge multiplexes
//! several over one socket); replies are written under a per-connection lock, so
//! a client that stops reading stalls only its own connection.

use crate::clients::{AgentClient, AgentClientStore};
use crate::error::AgentError;
use crate::protocol::{
    encode_frame, AgentCall, AgentHello, AgentHelloReply, AgentReply, FrameReader,
};
use crate::sock::send_all;
use crate::tools::AgentCallHandler;
use serde::Serialize;
use std::collections::HashMap;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;
use uuid::Uuid;

/// What a connection that is not paired is told, and what the bridge reports.
pub const NOT_PAIRED: &str = "This MCP client is not paired with Maine.";

/// Calls in flight on one connection at most; more are refused, not queued.
const MAX_CALLS_IN_FLIGHT: usize = 64;
/// A connection must say hello within this long.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

/// `$MAINE_AGENT_SOCKET`, else `$XDG_RUNTIME_DIR/maine/agent.sock`, else
/// `<temp dir>/maine-<uid>/agent.sock`.
pub fn default_socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("MAINE_AGENT_SOCKET").filter(|p| !p.is_empty()) {
        return PathBuf::from(path);
    }
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").filter(|p| !p.is_empty()) {
        return PathBuf::from(runtime).join("maine").join("agent.sock");
    }
    let uid = rustix::process::getuid().as_raw();
    std::env::temp_dir()
        .join(format!("maine-{uid}"))
        .join("agent.sock")
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

struct Connection {
    id: u64,
    /// Kept to shut the socket down from another thread.
    control: UnixStream,
    writer: Mutex<UnixStream>,
    client: Mutex<Option<AgentClient>>,
    closed: AtomicBool,
    in_flight: AtomicUsize,
}

impl Connection {
    fn client(&self) -> Option<AgentClient> {
        lock(&self.client).clone()
    }

    fn close(&self) -> bool {
        let first = !self.closed.swap(true, Ordering::AcqRel);
        if first {
            let _ = self.control.shutdown(std::net::Shutdown::Both);
        }
        first
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn send<T: Serialize>(&self, value: &T) -> bool {
        let Ok(frame) = encode_frame(value) else {
            return false;
        };
        if self.is_closed() {
            return false;
        }
        let writer = lock(&self.writer);
        if send_all(&writer, &frame).is_err() {
            drop(writer);
            self.close();
            return false;
        }
        true
    }
}

struct Inner {
    socket_path: PathBuf,
    clients: Arc<AgentClientStore>,
    handler: Arc<dyn AgentCallHandler>,
    running: AtomicBool,
    connections: Mutex<HashMap<u64, Arc<Connection>>>,
    next_id: AtomicUsize,
}

pub struct AgentServer {
    inner: Arc<Inner>,
    accept: Mutex<Option<Accepting>>,
}

/// The accept thread and a handle on its listening socket, to wake it.
struct Accepting {
    thread: JoinHandle<()>,
    listener: UnixListener,
}

impl AgentServer {
    pub fn new(
        socket_path: PathBuf,
        clients: Arc<AgentClientStore>,
        handler: Arc<dyn AgentCallHandler>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                socket_path,
                clients,
                handler,
                running: AtomicBool::new(false),
                connections: Mutex::new(HashMap::new()),
                next_id: AtomicUsize::new(1),
            }),
            accept: Mutex::new(None),
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.inner.socket_path
    }

    pub fn is_running(&self) -> bool {
        self.inner.running.load(Ordering::Acquire)
    }

    /// Starts listening. Creates the directory (0700) and the socket (0600),
    /// replacing a stale socket file but never one a live Maine serves.
    pub fn start(&self) -> Result<(), AgentError> {
        let mut accept = lock(&self.accept);
        if self.is_running() {
            return Ok(());
        }
        let path = &self.inner.socket_path;
        let directory = path
            .parent()
            .ok_or_else(|| AgentError::new("The agent socket path has no directory."))?;
        std::fs::create_dir_all(directory).map_err(|e| {
            AgentError::new(format!("Could not create the agent socket directory: {e}"))
        })?;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).map_err(
            |e| {
                AgentError::new(format!(
                    "Could not restrict the agent socket directory: {e}"
                ))
            },
        )?;
        let owner = std::fs::metadata(directory)
            .map(|m| m.uid())
            .unwrap_or(u32::MAX);
        if owner != rustix::process::getuid().as_raw() {
            return Err(AgentError::new(
                "The agent socket directory belongs to another user.",
            ));
        }
        if path.symlink_metadata().is_ok() {
            if UnixStream::connect(path).is_ok() {
                return Err(AgentError::new(
                    "Another copy of Maine is already serving agents.",
                ));
            }
            std::fs::remove_file(path).map_err(|e| {
                AgentError::new(format!("Could not remove the stale agent socket: {e}"))
            })?;
        }
        let listener = UnixListener::bind(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::InvalidInput {
                AgentError::new("The agent socket path is too long.")
            } else {
                AgentError::new(format!("Could not listen on the agent socket: {e}"))
            }
        })?;
        if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            let _ = std::fs::remove_file(path);
            return Err(AgentError::new(format!(
                "Could not restrict the agent socket: {e}"
            )));
        }
        let waker = listener.try_clone().map_err(|e| {
            let _ = std::fs::remove_file(path);
            AgentError::new(format!("Could not listen on the agent socket: {e}"))
        })?;
        self.inner.running.store(true, Ordering::Release);
        let inner = self.inner.clone();
        *accept = Some(Accepting {
            thread: std::thread::spawn(move || accept_loop(inner, listener)),
            listener: waker,
        });
        Ok(())
    }

    /// Stops listening, closes every connection and removes the socket file.
    pub fn stop(&self) {
        let mut accept = lock(&self.accept);
        if !self.inner.running.swap(false, Ordering::AcqRel) {
            return;
        }
        let open: Vec<Arc<Connection>> = lock(&self.inner.connections)
            .drain()
            .map(|(_, c)| c)
            .collect();
        for connection in open {
            connection.close();
        }
        if let Some(accepting) = accept.take() {
            // Wake the accept loop, which is blocked in accept(): shutting down
            // a listening socket makes it return, even if the socket file has
            // already been removed. The connect is a fallback.
            let _ = rustix::net::shutdown(&accepting.listener, rustix::net::Shutdown::Both);
            let _ = UnixStream::connect(&self.inner.socket_path);
            let _ = accepting.thread.join();
        }
        let _ = std::fs::remove_file(&self.inner.socket_path);
    }

    /// Ends every live connection of a client (revoked, or the master switch).
    pub fn disconnect(&self, client_id: Uuid) {
        let matching: Vec<Arc<Connection>> = lock(&self.inner.connections)
            .values()
            .filter(|connection| {
                connection
                    .client()
                    .is_some_and(|client| client.id == client_id)
            })
            .cloned()
            .collect();
        for connection in matching {
            connection.close();
        }
    }
}

impl Drop for AgentServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn accept_loop(inner: Arc<Inner>, listener: UnixListener) {
    loop {
        let incoming = listener.accept().map(|(stream, _)| stream);
        if !inner.running.load(Ordering::Acquire) {
            break;
        }
        let stream = match incoming {
            Ok(stream) => stream,
            Err(_) => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
        };
        // Only this user's processes get any further.
        let same_user = rustix::net::sockopt::socket_peercred(&stream)
            .map(|credentials| credentials.uid == rustix::process::getuid())
            .unwrap_or(false);
        if !same_user {
            continue;
        }
        let Ok(writer) = stream.try_clone() else {
            continue;
        };
        let Ok(control) = stream.try_clone() else {
            continue;
        };
        let id = inner.next_id.fetch_add(1, Ordering::Relaxed) as u64;
        let connection = Arc::new(Connection {
            id,
            control,
            writer: Mutex::new(writer),
            client: Mutex::new(None),
            closed: AtomicBool::new(false),
            in_flight: AtomicUsize::new(0),
        });
        lock(&inner.connections).insert(id, connection.clone());
        let inner = inner.clone();
        std::thread::spawn(move || serve(inner, connection, stream));
    }
}

fn serve(inner: Arc<Inner>, connection: Arc<Connection>, mut stream: UnixStream) {
    let _ = stream.set_read_timeout(Some(HELLO_TIMEOUT));
    let mut reader = FrameReader::new();
    let mut buffer = vec![0u8; 65_536];
    'read: loop {
        let count = match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        let Ok(frames) = reader.feed(&buffer[..count]) else {
            break;
        };
        for frame in frames {
            if !received(&inner, &connection, &stream, &frame) {
                break 'read;
            }
        }
    }
    connection.close();
    lock(&inner.connections).remove(&connection.id);
}

/// Handles one frame; false ends the connection.
fn received(
    inner: &Arc<Inner>,
    connection: &Arc<Connection>,
    stream: &UnixStream,
    frame: &[u8],
) -> bool {
    let Some(client) = connection.client() else {
        let paired = serde_json::from_slice::<AgentHello>(frame)
            .ok()
            .and_then(|hello| inner.clients.client_for_token(&hello.token).ok().flatten());
        let Some(client) = paired else {
            connection.send(&AgentHelloReply {
                ok: false,
                error: Some(NOT_PAIRED.into()),
            });
            return false;
        };
        *lock(&connection.client) = Some(client);
        // Authenticated: an idle bridge may now wait as long as it likes.
        let _ = stream.set_read_timeout(None);
        return connection.send(&AgentHelloReply {
            ok: true,
            error: None,
        });
    };
    let Ok(call) = serde_json::from_slice::<AgentCall>(frame) else {
        return false;
    };
    if connection.in_flight.fetch_add(1, Ordering::AcqRel) >= MAX_CALLS_IN_FLIGHT {
        connection.in_flight.fetch_sub(1, Ordering::AcqRel);
        connection.send(&AgentReply::failure(call.id, "too many concurrent calls"));
        return true;
    }
    let (inner, connection) = (inner.clone(), connection.clone());
    std::thread::spawn(move || {
        let reply = inner.handler.handle(call, &client);
        connection.in_flight.fetch_sub(1, Ordering::AcqRel);
        connection.send(&reply);
    });
    true
}
