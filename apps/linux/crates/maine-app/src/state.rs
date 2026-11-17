//! Process-wide application state shared by the QML bridge and the C ABI.

use maine_engine::Engine;
use std::sync::{Arc, Mutex, OnceLock};

static ENGINE: OnceLock<Arc<Mutex<Engine>>> = OnceLock::new();

pub fn init(engine: Engine) {
    let _ = ENGINE.set(Arc::new(Mutex::new(engine)));
}

pub fn engine() -> Arc<Mutex<Engine>> {
    ENGINE.get().expect("engine initialised in main").clone()
}

/// Runs `f` with the engine locked. A panic in another thread does not poison the app.
pub fn with_engine<R>(f: impl FnOnce(&mut Engine) -> R) -> R {
    let engine = engine();
    let mut guard = engine.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}
