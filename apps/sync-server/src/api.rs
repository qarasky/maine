//! HTTP surface of the sync server.
//!
//! The normative wire contract lives in `docs/sync-protocol.md` at the
//! repository root; keep the two in step. Nothing here ever reads a blob, a
//! token or a master password — the only secret the server touches is a
//! SHA-256 hash of a token.

use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use axum::body::Bytes;
use axum::extract::{FromRequestParts, Path, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use serde_json::json;

use crate::config::Config;
use crate::db::{Db, Expected, PutOutcome, Slot};
use crate::{hash, token};

const X_VERSION: HeaderName = HeaderName::from_static("x-maine-version");
const X_SIZE: HeaderName = HeaderName::from_static("x-maine-size");
const X_SHA256: HeaderName = HeaderName::from_static("x-maine-sha256");
const X_UPDATED_AT: HeaderName = HeaderName::from_static("x-maine-updated-at");
const X_CREATE_SECRET: HeaderName = HeaderName::from_static("x-maine-create-secret");

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Db>,
    pub cfg: Arc<Config>,
}

impl AppState {
    pub fn new(db: Db, cfg: Config) -> Self {
        Self {
            db: Arc::new(db),
            cfg: Arc::new(cfg),
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/vaults", post(create_vault))
        .route(
            "/v1/vault",
            get(get_vault).put(put_vault).delete(delete_vault),
        )
        .route("/v1/vault/versions", get(list_versions))
        .route("/v1/vault/versions/{version}", get(get_version))
        .route("/v1/vault/rotate", post(rotate_token))
        .fallback(not_found)
        .layer(middleware::from_fn(log_request))
        .with_state(state)
}

// -- handlers ---------------------------------------------------------------

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        name: "maine-sync-server",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn create_vault(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request,
) -> Result<Response, ApiError> {
    if let Some(secret) = state.cfg.create_secret.as_deref() {
        let provided = headers
            .get(X_CREATE_SECRET)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if !constant_time_eq(provided.as_bytes(), secret.as_bytes()) {
            return Err(ApiError::Forbidden(
                "creating slots requires the create secret",
            ));
        }
    }

    let body = read_body(&headers, request, state.cfg.max_blob).await?;
    let canonical = token::generate();
    let token_hash = token::hash_of(&canonical);
    let blob = (!body.is_empty()).then_some(&body[..]);
    let (slot, sha256) = state
        .db
        .create_slot(&token_hash, blob)
        .map_err(ApiError::Internal)?;
    tracing::info!(version = slot.version, size = slot.size, "slot created");

    let response = CreateResponse {
        token: token::display(&canonical),
        version: slot.version,
        etag: etag(slot.version),
        size: slot.size,
        sha256: sha256.map(|digest| hash::hex(&digest)),
    };
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

async fn get_vault(
    auth: Auth,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let slot = auth.0;
    if slot.version == 0 {
        let mut response = StatusCode::NO_CONTENT.into_response();
        fill_meta(response.headers_mut(), &slot, None, false);
        return Ok(response);
    }

    if let Some(values) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        && if_none_match_matches(values, slot.version)
    {
        let mut response = StatusCode::NOT_MODIFIED.into_response();
        fill_meta(response.headers_mut(), &slot, None, true);
        return Ok(response);
    }

    let blob = state
        .db
        .blob(slot.id, slot.version)
        .map_err(ApiError::Internal)?
        .ok_or_else(|| {
            ApiError::Internal(anyhow::anyhow!(
                "current version {} is missing from storage",
                slot.version
            ))
        })?;
    let sha256 = hash::sha256(&blob);
    let mut response = ([(header::CONTENT_TYPE, "application/octet-stream")], blob).into_response();
    fill_meta(response.headers_mut(), &slot, Some(&sha256), true);
    Ok(response)
}

async fn put_vault(
    auth: Auth,
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request,
) -> Result<Response, ApiError> {
    let slot = auth.0;
    let expected = match parse_precondition(&headers)? {
        Some(expected) => expected,
        // An unconditional write is allowed exactly once: the first version
        // of an empty slot. Everything after that needs a precondition, so a
        // blind retry can never silently overwrite someone else's edit.
        None if slot.version == 0 => Expected::Empty,
        None => return Err(ApiError::PreconditionRequired(slot)),
    };

    let body = read_body(&headers, request, state.cfg.max_blob).await?;
    if body.is_empty() {
        return Err(ApiError::BadRequest(
            "the vault body must not be empty; delete the slot instead".into(),
        ));
    }

    match state
        .db
        .put(slot.id, expected, &body, state.cfg.keep_versions)
        .map_err(ApiError::Internal)?
    {
        PutOutcome::Stored { version, sha256 } => {
            tracing::info!(version, size = body.len(), "vault stored");
            Ok((
                StatusCode::OK,
                Json(StoredResponse {
                    version,
                    etag: etag(version),
                    size: body.len() as i64,
                    sha256: hash::hex(&sha256),
                }),
            )
                .into_response())
        }
        PutOutcome::Conflict(current) => Err(ApiError::Conflict(current)),
        PutOutcome::NotEmpty(current) => Err(ApiError::PreconditionFailed(current)),
    }
}

async fn list_versions(
    auth: Auth,
    State(state): State<AppState>,
) -> Result<Json<VersionsResponse>, ApiError> {
    let versions = state.db.versions(auth.0.id).map_err(ApiError::Internal)?;
    Ok(Json(VersionsResponse {
        versions: versions
            .into_iter()
            .map(|entry| VersionInfo {
                version: entry.version,
                size: entry.size,
                sha256: hash::hex(&entry.sha256),
                created_at: entry.created_at,
            })
            .collect(),
    }))
}

async fn get_version(
    auth: Auth,
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> Result<Response, ApiError> {
    let version: i64 = raw
        .parse()
        .ok()
        .filter(|version| *version >= 1)
        .ok_or_else(|| ApiError::BadRequest(format!("{raw:?} is not a version number")))?;
    let blob = state
        .db
        .blob(auth.0.id, version)
        .map_err(ApiError::Internal)?
        .ok_or_else(|| ApiError::NotFound(format!("version {version} is not stored")))?;
    let sha256 = hash::sha256(&blob);
    let size = blob.len() as i64;
    let historical = Slot {
        id: auth.0.id,
        version,
        size,
        updated_at: None,
    };
    let mut response = ([(header::CONTENT_TYPE, "application/octet-stream")], blob).into_response();
    fill_meta(response.headers_mut(), &historical, Some(&sha256), true);
    Ok(response)
}

async fn rotate_token(
    auth: Auth,
    State(state): State<AppState>,
) -> Result<Json<TokenResponse>, ApiError> {
    let canonical = token::generate();
    let token_hash = token::hash_of(&canonical);
    state
        .db
        .rotate(auth.0.id, &token_hash)
        .map_err(ApiError::Internal)?;
    tracing::info!(
        slot = auth.0.id,
        "token rotated; the previous token is dead"
    );
    Ok(Json(TokenResponse {
        token: token::display(&canonical),
    }))
}

async fn delete_vault(auth: Auth, State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    state.db.delete(auth.0.id).map_err(ApiError::Internal)?;
    tracing::info!(slot = auth.0.id, "slot and all versions deleted");
    Ok(StatusCode::NO_CONTENT)
}

async fn not_found() -> ApiError {
    ApiError::NotFound("no such endpoint".into())
}

// -- authentication ---------------------------------------------------------

/// The authenticated slot. Extracting `Auth` is the only way to name a slot;
/// the token never travels further than this type.
pub struct Auth(pub Slot);

impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or(ApiError::Unauthorized)?;
        let (scheme, raw) = header.split_once(' ').ok_or(ApiError::Unauthorized)?;
        if !scheme.eq_ignore_ascii_case("bearer") {
            return Err(ApiError::Unauthorized);
        }
        let canonical = token::normalize(raw.trim()).ok_or(ApiError::Unauthorized)?;
        let token_hash = token::hash_of(&canonical);
        // A single indexed lookup on a local SQLite file. It is short enough
        // that a blocking pool would be ceremony, not safety.
        let slot = state
            .db
            .find_slot(&token_hash)
            .map_err(ApiError::Internal)?
            .ok_or(ApiError::Unauthorized)?;
        Ok(Auth(slot))
    }
}

// -- errors -----------------------------------------------------------------

pub enum ApiError {
    Unauthorized,
    Forbidden(&'static str),
    BadRequest(String),
    NotFound(String),
    PreconditionRequired(Slot),
    PreconditionFailed(Slot),
    Conflict(Slot),
    TooLarge(u64),
    Internal(anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::Unauthorized => {
                let mut response =
                    error_response(StatusCode::UNAUTHORIZED, "missing or invalid token");
                response
                    .headers_mut()
                    .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
                response
            }
            ApiError::Forbidden(message) => error_response(StatusCode::FORBIDDEN, message),
            ApiError::BadRequest(message) => error_response(StatusCode::BAD_REQUEST, message),
            ApiError::NotFound(message) => error_response(StatusCode::NOT_FOUND, message),
            ApiError::PreconditionRequired(slot) => slot_state_response(
                StatusCode::PRECONDITION_REQUIRED,
                "If-Match is required to update an existing vault",
                slot,
            ),
            ApiError::PreconditionFailed(slot) => slot_state_response(
                StatusCode::PRECONDITION_FAILED,
                "the slot already holds a vault",
                slot,
            ),
            ApiError::Conflict(slot) => slot_state_response(
                StatusCode::CONFLICT,
                "the vault moved on; reconcile and retry",
                slot,
            ),
            ApiError::TooLarge(limit) => error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("the body exceeds the {limit} byte limit"),
            ),
            ApiError::Internal(error) => {
                tracing::error!(error = ?error, "internal error");
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
            }
        }
    }
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    let message: String = message.into();
    (status, Json(json!({ "error": message }))).into_response()
}

/// 409/412/428 carry the current state of the slot so a client can reconcile
/// without another round trip.
fn slot_state_response(status: StatusCode, message: &'static str, slot: Slot) -> Response {
    let mut response = (
        status,
        Json(ConflictBody {
            error: message,
            version: slot.version,
            etag: etag(slot.version),
            size: slot.size,
            updated_at: slot.updated_at,
        }),
    )
        .into_response();
    response
        .headers_mut()
        .insert(X_VERSION, header_value(&slot.version.to_string()));
    response
}

// -- helpers ----------------------------------------------------------------

/// Read a request body up to `max` bytes, answering 413 when it does not fit.
async fn read_body(headers: &HeaderMap, request: Request, max: u64) -> Result<Bytes, ApiError> {
    if let Some(length) = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    {
        if length > max {
            return Err(ApiError::TooLarge(max));
        }
        if length == 0 {
            return Ok(Bytes::new());
        }
    }
    let limit = usize::try_from(max.saturating_add(1)).unwrap_or(usize::MAX);
    match axum::body::to_bytes(request.into_body(), limit).await {
        Ok(body) if body.len() as u64 <= max => Ok(body),
        _ => Err(ApiError::TooLarge(max)),
    }
}

/// Parse the write precondition, if any. `None` means "no precondition was
/// sent"; callers decide whether that is acceptable.
fn parse_precondition(headers: &HeaderMap) -> Result<Option<Expected>, ApiError> {
    let if_match = headers
        .get(header::IF_MATCH)
        .and_then(|value| value.to_str().ok());
    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok());

    match (if_match, if_none_match) {
        (Some(_), Some(_)) => Err(ApiError::BadRequest(
            "send only one of If-Match or If-None-Match".into(),
        )),
        (Some(raw), None) => {
            let raw = raw.trim();
            if raw == "*" {
                return Err(ApiError::BadRequest(
                    "If-Match: * is not supported; use If-None-Match: * to write the first \
                     version of an empty slot"
                        .into(),
                ));
            }
            let version = parse_etag(raw).ok_or_else(|| {
                ApiError::BadRequest(format!(
                    "unparsable If-Match value {raw:?}; expected \"v<number>\""
                ))
            })?;
            Ok(Some(Expected::Version(version)))
        }
        (None, Some(raw)) => {
            if raw.split(',').any(|part| part.trim() == "*") {
                Ok(Some(Expected::Empty))
            } else {
                Err(ApiError::BadRequest(
                    "only If-None-Match: * is supported".into(),
                ))
            }
        }
        (None, None) => Ok(None),
    }
}

fn if_none_match_matches(values: &str, version: i64) -> bool {
    values
        .split(',')
        .any(|part| part.trim() == "*" || parse_etag(part) == Some(version))
}

fn etag(version: i64) -> String {
    format!("\"v{version}\"")
}

/// Lenient ETag parsing: quotes and the weak marker are optional.
fn parse_etag(value: &str) -> Option<i64> {
    let value = value.trim();
    let value = value.strip_prefix("W/").unwrap_or(value);
    let value = value.strip_prefix('"').unwrap_or(value);
    let value = value.strip_suffix('"').unwrap_or(value);
    value
        .strip_prefix('v')
        .and_then(|digits| digits.parse().ok())
}

/// Metadata for blob responses, in both the 200 and 304 shapes.
fn fill_meta(headers: &mut HeaderMap, slot: &Slot, sha256: Option<&[u8; 32]>, with_etag: bool) {
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(X_VERSION, header_value(&slot.version.to_string()));
    headers.insert(X_SIZE, header_value(&slot.size.to_string()));
    if with_etag {
        headers.insert(header::ETAG, header_value(&etag(slot.version)));
    }
    if let Some(updated_at) = slot.updated_at {
        headers.insert(X_UPDATED_AT, header_value(&updated_at.to_string()));
    }
    if let Some(sha256) = sha256 {
        headers.insert(X_SHA256, header_value(&hash::hex(sha256)));
    }
}

/// Every value we build is digits, hex or an etag, so this cannot fail.
fn header_value(value: &str) -> HeaderValue {
    HeaderValue::from_str(value).expect("generated header values are ASCII")
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

/// One debug line per request: which route, which status, how long. Never the
/// token, the headers or the body.
async fn log_request(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let started = Instant::now();
    let response = next.run(request).await;
    tracing::debug!(
        method = %method,
        path = %path,
        status = response.status().as_u16(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "request"
    );
    response
}

// -- wire types -------------------------------------------------------------

#[derive(Serialize)]
struct Health {
    status: &'static str,
    name: &'static str,
    version: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateResponse {
    token: String,
    version: i64,
    etag: String,
    size: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredResponse {
    version: i64,
    etag: String,
    size: i64,
    sha256: String,
}

#[derive(Serialize)]
struct TokenResponse {
    token: String,
}

#[derive(Serialize)]
struct VersionsResponse {
    versions: Vec<VersionInfo>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VersionInfo {
    version: i64,
    size: i64,
    sha256: String,
    created_at: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConflictBody {
    error: &'static str,
    version: i64,
    etag: String,
    size: i64,
    updated_at: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::{etag, if_none_match_matches, parse_etag};

    #[test]
    fn etags_round_trip() {
        assert_eq!(etag(7), "\"v7\"");
        assert_eq!(parse_etag("\"v7\""), Some(7));
        assert_eq!(parse_etag("v7"), Some(7));
        assert_eq!(parse_etag("W/\"v7\""), Some(7));
        assert_eq!(parse_etag(" v0 "), Some(0));
        assert_eq!(parse_etag("\"vx\""), None);
        assert_eq!(parse_etag("something"), None);
    }

    #[test]
    fn if_none_match_understands_lists_and_stars() {
        assert!(if_none_match_matches("\"v3\"", 3));
        assert!(if_none_match_matches("\"v1\", W/\"v3\"", 3));
        assert!(if_none_match_matches("*", 3));
        assert!(!if_none_match_matches("\"v2\"", 3));
    }
}
