//! The wire contract from `docs/sync-protocol.md` behind a trait, so the
//! engine and coordinator can be tested without a network, plus the real
//! blocking HTTP implementation.
//!
//! Nothing here logs. Tokens appear only in the `Authorization` header of the
//! request they authenticate; error values never carry a URL, header or body.

use crate::error::SyncError;
use crate::url::endpoint;
use serde::Deserialize;
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;

/// Largest vault body the client will read from a server (the server's own
/// default limit is 64 MiB).
const MAX_BLOB_BYTES: u64 = 256 * 1024 * 1024;
/// Largest JSON document the client will read.
const MAX_JSON_BYTES: u64 = 1024 * 1024;

/// One slot's server-side state, from response headers or JSON.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SlotState {
    pub version: u64,
    pub size: u64,
    pub sha256: Option<String>,
    /// Unix seconds, from `X-Maine-Updated-At`.
    pub updated_at: Option<u64>,
}

impl SlotState {
    pub fn new(version: u64) -> Self {
        Self {
            version,
            ..Self::default()
        }
    }

    pub fn with(version: u64, size: u64, sha256: impl Into<String>) -> Self {
        Self {
            version,
            size,
            sha256: Some(sha256.into()),
            updated_at: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchResult {
    /// `304`: the caller's last version is still current.
    NotModified(SlotState),
    /// `204`: the slot holds nothing yet.
    Empty(SlotState),
    /// `200` to a `HEAD`: metadata only.
    Current(SlotState),
    /// `200`: the current blob.
    Blob(Vec<u8>, SlotState),
}

/// One row of `GET /v1/vault/versions`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct VersionInfo {
    pub version: u64,
    pub size: u64,
    pub sha256: String,
    #[serde(rename = "createdAt")]
    pub created_at: u64,
}

/// The protocol, one method per request. Blocking: the coordinator runs it on
/// its own thread. `server` is a normalized base URL (`normalize_server_url`).
pub trait Transport: Send + Sync {
    /// `POST /v1/vaults`. The token comes back exactly once.
    fn create_vault(
        &self,
        server: &str,
        create_secret: Option<&str>,
        body: &[u8],
    ) -> Result<(String, SlotState), SyncError>;

    /// `GET /v1/vault`. `if_none_match` is sent only when it is `Some(n)`, `n > 0`.
    fn fetch_vault(
        &self,
        server: &str,
        token: &str,
        if_none_match: Option<u64>,
    ) -> Result<FetchResult, SyncError>;

    /// `HEAD /v1/vault`.
    fn head_vault(
        &self,
        server: &str,
        token: &str,
        if_none_match: Option<u64>,
    ) -> Result<FetchResult, SyncError>;

    /// `PUT /v1/vault`: `If-Match: "v<n>"` for `Some(n)`, `If-None-Match: *`
    /// for `None`. `timeout` overrides the transport's request timeout for
    /// this one call (used for the bounded pushes before a workspace switch or
    /// quit); `None` keeps the default.
    fn put_vault(
        &self,
        server: &str,
        token: &str,
        body: &[u8],
        expected_version: Option<u64>,
        timeout: Option<Duration>,
    ) -> Result<SlotState, SyncError>;

    /// `POST /v1/vault/rotate`: returns the new token.
    fn rotate_token(&self, server: &str, token: &str) -> Result<String, SyncError>;

    /// `DELETE /v1/vault`.
    fn delete_vault(&self, server: &str, token: &str) -> Result<(), SyncError>;

    /// `GET /v1/vault/versions`, newest first. No UI uses this yet.
    fn list_versions(&self, server: &str, token: &str) -> Result<Vec<VersionInfo>, SyncError>;

    /// `GET /v1/vault/versions/{n}`. `404` becomes `SyncError::NotFound`.
    fn fetch_version(
        &self,
        server: &str,
        token: &str,
        version: u64,
    ) -> Result<(Vec<u8>, SlotState), SyncError>;
}

impl<T: Transport + ?Sized> Transport for Arc<T> {
    fn create_vault(
        &self,
        server: &str,
        create_secret: Option<&str>,
        body: &[u8],
    ) -> Result<(String, SlotState), SyncError> {
        (**self).create_vault(server, create_secret, body)
    }
    fn fetch_vault(
        &self,
        server: &str,
        token: &str,
        if_none_match: Option<u64>,
    ) -> Result<FetchResult, SyncError> {
        (**self).fetch_vault(server, token, if_none_match)
    }
    fn head_vault(
        &self,
        server: &str,
        token: &str,
        if_none_match: Option<u64>,
    ) -> Result<FetchResult, SyncError> {
        (**self).head_vault(server, token, if_none_match)
    }
    fn put_vault(
        &self,
        server: &str,
        token: &str,
        body: &[u8],
        expected_version: Option<u64>,
        timeout: Option<Duration>,
    ) -> Result<SlotState, SyncError> {
        (**self).put_vault(server, token, body, expected_version, timeout)
    }
    fn rotate_token(&self, server: &str, token: &str) -> Result<String, SyncError> {
        (**self).rotate_token(server, token)
    }
    fn delete_vault(&self, server: &str, token: &str) -> Result<(), SyncError> {
        (**self).delete_vault(server, token)
    }
    fn list_versions(&self, server: &str, token: &str) -> Result<Vec<VersionInfo>, SyncError> {
        (**self).list_versions(server, token)
    }
    fn fetch_version(
        &self,
        server: &str,
        token: &str,
        version: u64,
    ) -> Result<(Vec<u8>, SlotState), SyncError> {
        (**self).fetch_version(server, token, version)
    }
}

// MARK: - HTTP

/// Blocking `ureq` transport (rustls). One agent per transport, no cookies, no
/// redirects at all (a redirect could carry the token to another host), a
/// generic `User-Agent: Maine-Sync`, and per-request timeouts.
pub struct HttpTransport {
    agent: ureq::Agent,
    request_timeout: Duration,
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpTransport {
    /// The macOS defaults: 10 s without progress, 30 s for a whole request.
    pub fn new() -> Self {
        Self::with_request_timeout(Duration::from_secs(10))
    }

    pub fn with_request_timeout(request_timeout: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .user_agent("Maine-Sync")
            .timeout_connect(Some(request_timeout))
            .timeout_per_read(Some(request_timeout))
            .timeout_global(Some(request_timeout * 3))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            request_timeout,
        }
    }

    pub fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    /// `GET /v1/health`: a cheap "is this a reachable server" check for the
    /// create and join forms. No token is sent.
    pub fn health(&self, server: &str) -> Result<(), SyncError> {
        let response = finish(
            self.agent
                .get(endpoint(server, "v1/health"))
                .header("Accept", "application/json")
                .call(),
        )?;
        expect(response, &[200]).map(drop)
    }

    fn fetch(
        &self,
        head: bool,
        server: &str,
        token: &str,
        if_none_match: Option<u64>,
    ) -> Result<FetchResult, SyncError> {
        let url = endpoint(server, "v1/vault");
        let tag = if_none_match.filter(|v| *v > 0).map(etag);
        let response = if head {
            let mut request = self.agent.head(url).header("Accept", "application/json");
            request = request.header("Authorization", bearer(token));
            if let Some(tag) = &tag {
                request = request.header("If-None-Match", tag);
            }
            finish(request.call())?
        } else {
            let mut request = self.agent.get(url).header("Accept", "application/json");
            request = request.header("Authorization", bearer(token));
            if let Some(tag) = &tag {
                request = request.header("If-None-Match", tag);
            }
            finish(request.call())?
        };
        match response.status().as_u16() {
            304 => Ok(FetchResult::NotModified(state_from(&response))),
            204 => Ok(FetchResult::Empty(state_from(&response))),
            200 => {
                let state = state_from(&response);
                if head {
                    Ok(FetchResult::Current(state))
                } else {
                    let bytes = read_body(response, MAX_BLOB_BYTES)?;
                    Ok(FetchResult::Blob(bytes, state))
                }
            }
            _ => Err(error_for(response)),
        }
    }
}

impl Transport for HttpTransport {
    fn create_vault(
        &self,
        server: &str,
        create_secret: Option<&str>,
        body: &[u8],
    ) -> Result<(String, SlotState), SyncError> {
        let mut request = self
            .agent
            .post(endpoint(server, "v1/vaults"))
            .header("Accept", "application/json")
            .header("Content-Type", "application/octet-stream");
        if let Some(secret) = create_secret.filter(|s| !s.is_empty()) {
            request = request.header("X-Maine-Create-Secret", secret);
        }
        let response = expect(finish(request.send(body))?, &[201])?;
        let created: CreatedResponse = decode(response)?;
        Ok((
            created.token,
            SlotState {
                version: created.version,
                size: created.size,
                sha256: created.sha256,
                updated_at: None,
            },
        ))
    }

    fn fetch_vault(
        &self,
        server: &str,
        token: &str,
        if_none_match: Option<u64>,
    ) -> Result<FetchResult, SyncError> {
        self.fetch(false, server, token, if_none_match)
    }

    fn head_vault(
        &self,
        server: &str,
        token: &str,
        if_none_match: Option<u64>,
    ) -> Result<FetchResult, SyncError> {
        self.fetch(true, server, token, if_none_match)
    }

    fn put_vault(
        &self,
        server: &str,
        token: &str,
        body: &[u8],
        expected_version: Option<u64>,
        timeout: Option<Duration>,
    ) -> Result<SlotState, SyncError> {
        let mut request = self
            .agent
            .put(endpoint(server, "v1/vault"))
            .header("Accept", "application/json")
            .header("Authorization", bearer(token))
            .header("Content-Type", "application/octet-stream");
        request = match expected_version {
            Some(version) => request.header("If-Match", etag(version)),
            None => request.header("If-None-Match", "*"),
        };
        let response = if let Some(limit) = timeout {
            finish(
                request
                    .config()
                    .timeout_global(Some(limit))
                    .build()
                    .send(body),
            )?
        } else {
            finish(request.send(body))?
        };
        let response = expect(response, &[200])?;
        let stored: StoredResponse = decode(response)?;
        Ok(SlotState {
            version: stored.version,
            size: stored.size,
            sha256: stored.sha256,
            updated_at: None,
        })
    }

    fn rotate_token(&self, server: &str, token: &str) -> Result<String, SyncError> {
        let response = finish(
            self.agent
                .post(endpoint(server, "v1/vault/rotate"))
                .header("Accept", "application/json")
                .header("Authorization", bearer(token))
                .send_empty(),
        )?;
        let rotated: TokenResponse = decode(expect(response, &[200])?)?;
        Ok(rotated.token)
    }

    fn delete_vault(&self, server: &str, token: &str) -> Result<(), SyncError> {
        let response = finish(
            self.agent
                .delete(endpoint(server, "v1/vault"))
                .header("Accept", "application/json")
                .header("Authorization", bearer(token))
                .call(),
        )?;
        expect(response, &[204, 200]).map(drop)
    }

    fn list_versions(&self, server: &str, token: &str) -> Result<Vec<VersionInfo>, SyncError> {
        let response = finish(
            self.agent
                .get(endpoint(server, "v1/vault/versions"))
                .header("Accept", "application/json")
                .header("Authorization", bearer(token))
                .call(),
        )?;
        let list: VersionsResponse = decode(expect(response, &[200])?)?;
        Ok(list.versions)
    }

    fn fetch_version(
        &self,
        server: &str,
        token: &str,
        version: u64,
    ) -> Result<(Vec<u8>, SlotState), SyncError> {
        let response = finish(
            self.agent
                .get(endpoint(server, &format!("v1/vault/versions/{version}")))
                .header("Accept", "application/json")
                .header("Authorization", bearer(token))
                .call(),
        )?;
        let response = expect(response, &[200])?;
        let state = state_from(&response);
        Ok((read_body(response, MAX_BLOB_BYTES)?, state))
    }
}

// MARK: - Plumbing

type Response = ureq::http::Response<ureq::Body>;

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

fn etag(version: u64) -> String {
    format!("\"v{version}\"")
}

/// Maps a transport-level failure to the user-facing offline wording. The
/// underlying error is dropped on purpose: it can name the host.
fn finish(result: Result<Response, ureq::Error>) -> Result<Response, SyncError> {
    result.map_err(|error| match error {
        ureq::Error::Timeout(_) => {
            SyncError::Offline("The sync server took too long to answer.".into())
        }
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => {
            SyncError::Offline("The sync server could not be reached.".into())
        }
        ureq::Error::Io(io) => match io.kind() {
            ErrorKind::TimedOut | ErrorKind::WouldBlock => {
                SyncError::Offline("The sync server took too long to answer.".into())
            }
            ErrorKind::NetworkUnreachable | ErrorKind::NetworkDown => {
                SyncError::Offline("No network connection.".into())
            }
            _ => SyncError::Offline("The sync server could not be reached.".into()),
        },
        ureq::Error::Tls(_) | ureq::Error::Pem(_) | ureq::Error::Rustls(_) => {
            SyncError::Offline("Could not make a secure connection to the sync server.".into())
        }
        ureq::Error::BadUri(_) => SyncError::Malformed("The server address is not valid.".into()),
        ureq::Error::Http(_) => SyncError::Malformed(
            "The request could not be built. Check the server address and token.".into(),
        ),
        ureq::Error::BodyExceedsLimit(_) => {
            SyncError::Malformed("The server's response was too large.".into())
        }
        ureq::Error::Protocol(_) => {
            SyncError::Malformed("The server sent an unexpected response.".into())
        }
        _ => SyncError::Offline("The sync server could not be reached.".into()),
    })
}

/// Passes the response through when its status is in `statuses`, otherwise
/// turns it into the matching `SyncError`.
fn expect(response: Response, statuses: &[u16]) -> Result<Response, SyncError> {
    if statuses.contains(&response.status().as_u16()) {
        Ok(response)
    } else {
        Err(error_for(response))
    }
}

fn error_for(response: Response) -> SyncError {
    let status = response.status().as_u16();
    let version = header_u64(&response, "X-Maine-Version");
    let message = if (300..400).contains(&status) {
        "The sync server redirected the request. Use the address it redirects to.".to_string()
    } else {
        read_body(response, 64 * 1024)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<ErrorBody>(&bytes).ok())
            .map(|body| body.error)
            .unwrap_or_else(|| format!("The server answered with status {status}."))
    };
    match status {
        401 => SyncError::Unauthorized,
        403 => SyncError::Forbidden(message),
        404 => SyncError::NotFound(message),
        409 | 412 => SyncError::Conflict {
            current_version: version.unwrap_or(0),
        },
        413 => SyncError::TooLarge,
        428 => SyncError::PreconditionRequired {
            current_version: version.unwrap_or(0),
        },
        _ => SyncError::Server { status, message },
    }
}

fn header_str<'a>(response: &'a Response, name: &str) -> Option<&'a str> {
    response.headers().get(name).and_then(|v| v.to_str().ok())
}

fn header_u64(response: &Response, name: &str) -> Option<u64> {
    header_str(response, name).and_then(|v| v.trim().parse().ok())
}

fn state_from(response: &Response) -> SlotState {
    SlotState {
        version: header_u64(response, "X-Maine-Version").unwrap_or(0),
        size: header_u64(response, "X-Maine-Size").unwrap_or(0),
        sha256: header_str(response, "X-Maine-SHA256").map(str::to_string),
        updated_at: header_u64(response, "X-Maine-Updated-At"),
    }
}

fn read_body(mut response: Response, limit: u64) -> Result<Vec<u8>, SyncError> {
    response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|error| match error {
            ureq::Error::BodyExceedsLimit(_) => {
                SyncError::Malformed("The server's response was too large.".into())
            }
            ureq::Error::Timeout(_) => {
                SyncError::Offline("The sync server took too long to answer.".into())
            }
            _ => SyncError::Offline("The connection to the sync server was interrupted.".into()),
        })
}

fn decode<T: for<'de> Deserialize<'de>>(response: Response) -> Result<T, SyncError> {
    let bytes = read_body(response, MAX_JSON_BYTES)?;
    serde_json::from_slice(&bytes)
        .map_err(|_| SyncError::Malformed("The server sent an unexpected response.".into()))
}

#[derive(Deserialize)]
struct CreatedResponse {
    token: String,
    version: u64,
    #[serde(default)]
    size: u64,
    sha256: Option<String>,
}

#[derive(Deserialize)]
struct StoredResponse {
    version: u64,
    #[serde(default)]
    size: u64,
    sha256: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    token: String,
}

#[derive(Deserialize)]
struct VersionsResponse {
    versions: Vec<VersionInfo>,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
