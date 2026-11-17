//! The bridge's connection to the app: hello first, then calls multiplexed by
//! id so parallel tool calls can share one socket. Blocking; one reader thread
//! delivers replies.

use crate::protocol::{
    encode_frame, AgentCall, AgentHello, AgentHelloReply, AgentReply, FrameReader,
};
use crate::sock::send_all;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientError {
    /// Nothing is listening.
    NotRunning,
    /// The connection was already closed; the request was never sent.
    NotSent,
    /// The connection closed after the request was sent: it may have run.
    Disconnected,
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotRunning => "nothing is listening on the agent socket",
            Self::NotSent => "the agent connection was closed before the request was sent",
            Self::Disconnected => "the agent connection closed after the request was sent",
        })
    }
}

impl std::error::Error for ClientError {}

#[derive(Default)]
struct Shared {
    pending: HashMap<i64, Sender<AgentReply>>,
    hello_waiter: Option<Sender<AgentHelloReply>>,
    next_id: i64,
    closed: bool,
}

pub struct AgentSocketClient {
    stream: UnixStream,
    writer: Mutex<()>,
    shared: Arc<Mutex<Shared>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Marks the connection closed and wakes everyone waiting on it.
fn fail(shared: &Mutex<Shared>, stream: &UnixStream) {
    let mut state = lock(shared);
    if state.closed {
        return;
    }
    state.closed = true;
    // Dropping the senders makes every waiting `recv` return an error.
    state.pending.clear();
    state.hello_waiter = None;
    drop(state);
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

impl AgentSocketClient {
    pub fn connect(path: &Path) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(path).map_err(|_| ClientError::NotRunning)?;
        let mut read_side = stream.try_clone().map_err(|_| ClientError::NotRunning)?;
        let shared = Arc::new(Mutex::new(Shared {
            next_id: 1,
            ..Shared::default()
        }));
        let reader_shared = shared.clone();
        let reader_stream = stream.try_clone().map_err(|_| ClientError::NotRunning)?;
        std::thread::spawn(move || {
            let mut reader = FrameReader::new();
            let mut buffer = vec![0u8; 65_536];
            loop {
                let count = match read_side.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => count,
                };
                let Ok(frames) = reader.feed(&buffer[..count]) else {
                    break;
                };
                for frame in frames {
                    deliver(&reader_shared, &frame);
                }
            }
            fail(&reader_shared, &reader_stream);
        });
        Ok(Self {
            stream,
            writer: Mutex::new(()),
            shared,
        })
    }

    pub fn is_closed(&self) -> bool {
        lock(&self.shared).closed
    }

    pub fn hello(&self, hello: &AgentHello) -> Result<AgentHelloReply, ClientError> {
        let frame = encode_frame(hello).map_err(|_| ClientError::NotSent)?;
        let (sender, receiver) = channel();
        {
            let mut state = lock(&self.shared);
            if state.closed {
                return Err(ClientError::NotSent);
            }
            state.hello_waiter = Some(sender);
        }
        self.write(&frame);
        receiver.recv().map_err(|_| ClientError::Disconnected)
    }

    pub fn call(&self, tool: &str, arguments: Value) -> Result<AgentReply, ClientError> {
        let (sender, receiver) = channel();
        let frame = {
            let mut state = lock(&self.shared);
            if state.closed {
                return Err(ClientError::NotSent);
            }
            let id = state.next_id;
            state.next_id += 1;
            let call = AgentCall {
                id,
                tool: tool.to_string(),
                arguments,
            };
            let frame = encode_frame(&call).map_err(|_| ClientError::NotSent)?;
            state.pending.insert(id, sender);
            frame
        };
        self.write(&frame);
        receiver.recv().map_err(|_| ClientError::Disconnected)
    }

    pub fn close(&self) {
        fail(&self.shared, &self.stream);
    }

    fn write(&self, frame: &[u8]) {
        // Never under the state lock: a slow peer must not stall the reader.
        let _serialized = lock(&self.writer);
        if send_all(&self.stream, frame).is_err() {
            fail(&self.shared, &self.stream);
        }
    }
}

impl Drop for AgentSocketClient {
    fn drop(&mut self) {
        self.close();
    }
}

fn deliver(shared: &Mutex<Shared>, frame: &[u8]) {
    let mut state = lock(shared);
    if let Some(waiter) = state.hello_waiter.take() {
        if let Ok(reply) = serde_json::from_slice::<AgentHelloReply>(frame) {
            drop(state);
            let _ = waiter.send(reply);
            return;
        }
        state.hello_waiter = Some(waiter);
    }
    let Ok(reply) = serde_json::from_slice::<AgentReply>(frame) else {
        return;
    };
    let Some(waiter) = state.pending.remove(&reply.id) else {
        return;
    };
    drop(state);
    let _ = waiter.send(reply);
}
