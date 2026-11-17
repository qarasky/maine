//! Test support shared by this crate's tests and the app's: a programmable
//! stand-in for the sync server and a clock you move by hand. Not used by
//! production code.

use crate::error::SyncError;
use crate::time::{Clock, SystemClock};
use crate::transport::{FetchResult, SlotState, Transport, VersionInfo};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One request the fake saw, with what a test may want to assert on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    Create {
        create_secret: Option<String>,
        body: Vec<u8>,
    },
    Fetch {
        token: String,
        if_none_match: Option<u64>,
    },
    Head {
        token: String,
        if_none_match: Option<u64>,
    },
    Put {
        token: String,
        body: Vec<u8>,
        expected_version: Option<u64>,
        timeout: Option<Duration>,
    },
    Rotate {
        token: String,
    },
    Delete {
        token: String,
    },
    ListVersions,
    FetchVersion(u64),
}

impl Call {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Create { .. } => "create",
            Self::Fetch { .. } => "fetch",
            Self::Head { .. } => "head",
            Self::Put { .. } => "put",
            Self::Rotate { .. } => "rotate",
            Self::Delete { .. } => "delete",
            Self::ListVersions => "versions",
            Self::FetchVersion(_) => "version",
        }
    }
}

#[derive(Default)]
struct Inner {
    creates: VecDeque<Result<(String, SlotState), SyncError>>,
    /// Shared by `fetch` and `head`, like the Swift fake.
    fetches: VecDeque<Result<FetchResult, SyncError>>,
    puts: VecDeque<Result<SlotState, SyncError>>,
    rotates: VecDeque<Result<String, SyncError>>,
    deletes: VecDeque<Result<(), SyncError>>,
    versions: VecDeque<Result<Vec<VersionInfo>, SyncError>>,
    version_blobs: VecDeque<Result<(Vec<u8>, SlotState), SyncError>>,
    calls: Vec<Call>,
}

/// Every call consumes the next queued result, so a test describes exactly
/// what the server would answer. An empty queue answers `Malformed`.
#[derive(Default)]
pub struct FakeTransport {
    inner: Mutex<Inner>,
}

impl FakeTransport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enqueue_create(&self, result: Result<(String, SlotState), SyncError>) {
        self.inner.lock().unwrap().creates.push_back(result);
    }
    pub fn enqueue_fetch(&self, result: Result<FetchResult, SyncError>) {
        self.inner.lock().unwrap().fetches.push_back(result);
    }
    pub fn enqueue_put(&self, result: Result<SlotState, SyncError>) {
        self.inner.lock().unwrap().puts.push_back(result);
    }
    pub fn enqueue_rotate(&self, result: Result<String, SyncError>) {
        self.inner.lock().unwrap().rotates.push_back(result);
    }
    pub fn enqueue_delete(&self, result: Result<(), SyncError>) {
        self.inner.lock().unwrap().deletes.push_back(result);
    }
    pub fn enqueue_versions(&self, result: Result<Vec<VersionInfo>, SyncError>) {
        self.inner.lock().unwrap().versions.push_back(result);
    }
    pub fn enqueue_version_blob(&self, result: Result<(Vec<u8>, SlotState), SyncError>) {
        self.inner.lock().unwrap().version_blobs.push_back(result);
    }

    pub fn calls(&self) -> Vec<Call> {
        self.inner.lock().unwrap().calls.clone()
    }

    pub fn call_names(&self) -> Vec<&'static str> {
        self.calls().iter().map(Call::name).collect()
    }

    pub fn clear_calls(&self) {
        self.inner.lock().unwrap().calls.clear();
    }

    fn pop<T>(
        &self,
        call: Call,
        queue: impl FnOnce(&mut Inner) -> &mut VecDeque<Result<T, SyncError>>,
        what: &str,
    ) -> Result<T, SyncError> {
        let mut inner = self.inner.lock().unwrap();
        inner.calls.push(call);
        queue(&mut inner)
            .pop_front()
            .unwrap_or_else(|| Err(SyncError::Malformed(format!("no {what} queued"))))
    }
}

impl Transport for FakeTransport {
    fn create_vault(
        &self,
        _server: &str,
        create_secret: Option<&str>,
        body: &[u8],
    ) -> Result<(String, SlotState), SyncError> {
        self.pop(
            Call::Create {
                create_secret: create_secret.map(str::to_string),
                body: body.to_vec(),
            },
            |i| &mut i.creates,
            "create",
        )
    }

    fn fetch_vault(
        &self,
        _server: &str,
        token: &str,
        if_none_match: Option<u64>,
    ) -> Result<FetchResult, SyncError> {
        self.pop(
            Call::Fetch {
                token: token.to_string(),
                if_none_match,
            },
            |i| &mut i.fetches,
            "fetch",
        )
    }

    fn head_vault(
        &self,
        _server: &str,
        token: &str,
        if_none_match: Option<u64>,
    ) -> Result<FetchResult, SyncError> {
        self.pop(
            Call::Head {
                token: token.to_string(),
                if_none_match,
            },
            |i| &mut i.fetches,
            "head",
        )
    }

    fn put_vault(
        &self,
        _server: &str,
        token: &str,
        body: &[u8],
        expected_version: Option<u64>,
        timeout: Option<Duration>,
    ) -> Result<SlotState, SyncError> {
        self.pop(
            Call::Put {
                token: token.to_string(),
                body: body.to_vec(),
                expected_version,
                timeout,
            },
            |i| &mut i.puts,
            "put",
        )
    }

    fn rotate_token(&self, _server: &str, token: &str) -> Result<String, SyncError> {
        self.pop(
            Call::Rotate {
                token: token.to_string(),
            },
            |i| &mut i.rotates,
            "rotate",
        )
    }

    fn delete_vault(&self, _server: &str, token: &str) -> Result<(), SyncError> {
        self.pop(
            Call::Delete {
                token: token.to_string(),
            },
            |i| &mut i.deletes,
            "delete",
        )
    }

    fn list_versions(&self, _server: &str, _token: &str) -> Result<Vec<VersionInfo>, SyncError> {
        self.pop(Call::ListVersions, |i| &mut i.versions, "versions")
    }

    fn fetch_version(
        &self,
        _server: &str,
        _token: &str,
        version: u64,
    ) -> Result<(Vec<u8>, SlotState), SyncError> {
        self.pop(
            Call::FetchVersion(version),
            |i| &mut i.version_blobs,
            "version blob",
        )
    }
}

/// A clock that only moves when told to.
pub struct ManualClock {
    base: Instant,
    offset: Mutex<Duration>,
    unix_base: f64,
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl ManualClock {
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
            offset: Mutex::new(Duration::ZERO),
            unix_base: SystemClock.unix_seconds(),
        }
    }

    pub fn advance(&self, by: Duration) {
        *self.offset.lock().unwrap() += by;
    }

    pub fn elapsed(&self) -> Duration {
        *self.offset.lock().unwrap()
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        self.base + *self.offset.lock().unwrap()
    }

    fn unix_seconds(&self) -> f64 {
        self.unix_base + self.offset.lock().unwrap().as_secs_f64()
    }
}
