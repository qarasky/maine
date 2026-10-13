//! End-to-end HTTP tests: a real server on an ephemeral loopback port, real
//! requests, real SQLite. These cover the contract in `docs/sync-protocol.md`.

use std::net::SocketAddr;

use maine_sync_server::api::{self, AppState};
use maine_sync_server::config::Config;
use maine_sync_server::db::Db;
use maine_sync_server::hash;
use serde_json::Value;

struct TestServer {
    addr: SocketAddr,
    client: reqwest::Client,
    _dir: tempfile::TempDir,
}

impl TestServer {
    async fn start(keep_versions: u32, max_blob: u64, create_secret: Option<&str>) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = Config {
            bind: "127.0.0.1:0".parse().expect("socket addr"),
            data_dir: dir.path().to_path_buf(),
            keep_versions,
            max_blob,
            create_secret: create_secret.map(str::to_owned),
        };
        let db = Db::open(&dir.path().join("sync.sqlite3")).expect("database");
        let app = api::router(AppState::new(db, config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            addr,
            client: reqwest::Client::new(),
            _dir: dir,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    async fn create(&self, body: Option<&[u8]>, secret: Option<&str>) -> reqwest::Response {
        let mut request = self.client.post(self.url("/v1/vaults"));
        if let Some(body) = body {
            request = request.body(body.to_vec());
        }
        if let Some(secret) = secret {
            request = request.header("x-maine-create-secret", secret);
        }
        request.send().await.expect("create request")
    }

    async fn get(&self, token: &str, headers: &[(&str, &str)]) -> reqwest::Response {
        let mut request = self.client.get(self.url("/v1/vault")).bearer_auth(token);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.send().await.expect("get request")
    }

    async fn put(&self, token: &str, body: &[u8], headers: &[(&str, &str)]) -> reqwest::Response {
        let mut request = self
            .client
            .put(self.url("/v1/vault"))
            .bearer_auth(token)
            .body(body.to_vec());
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.send().await.expect("put request")
    }

    async fn versions(&self, token: &str) -> reqwest::Response {
        self.client
            .get(self.url("/v1/vault/versions"))
            .bearer_auth(token)
            .send()
            .await
            .expect("versions request")
    }
}

async fn create_token(server: &TestServer, body: Option<&[u8]>) -> String {
    let response = server.create(body, None).await;
    assert_eq!(response.status(), 201);
    let json: Value = response.json().await.expect("create JSON");
    json["token"].as_str().expect("token field").to_owned()
}

async fn body_text(response: reqwest::Response) -> String {
    let bytes = response.bytes().await.expect("body");
    String::from_utf8(bytes.to_vec()).expect("utf8 body")
}

#[tokio::test]
async fn health_is_public() {
    let server = TestServer::start(32, 1 << 20, None).await;
    let response = server
        .client
        .get(server.url("/v1/health"))
        .send()
        .await
        .expect("health");
    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.expect("health JSON");
    assert_eq!(json["status"], "ok");
    assert_eq!(json["name"], "maine-sync-server");
}

#[tokio::test]
async fn an_empty_slot_reports_version_zero_and_no_content() {
    let server = TestServer::start(32, 1 << 20, None).await;
    let token = create_token(&server, None).await;
    assert!(token.starts_with("maine-"));

    let response = server.get(&token, &[]).await;
    assert_eq!(response.status(), 204);
    assert_eq!(response.headers()["x-maine-version"], "0");
}

#[tokio::test]
async fn create_with_a_body_stores_the_first_version() {
    let server = TestServer::start(32, 1 << 20, None).await;
    let vault = b"opaque vault bytes, version one";
    let token = create_token(&server, Some(vault)).await;

    let response = server.get(&token, &[]).await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["etag"], "\"v1\"");
    assert_eq!(response.headers()["x-maine-version"], "1");
    assert_eq!(
        response.headers()["x-maine-sha256"],
        hash::hex(&hash::sha256(vault))
    );
    assert_eq!(body_text(response).await.as_bytes(), vault);
}

#[tokio::test]
async fn conditional_get_avoids_the_round_trip_body() {
    let server = TestServer::start(32, 1 << 20, None).await;
    let token = create_token(&server, Some(b"vault one")).await;

    let fresh = server.get(&token, &[("if-none-match", "\"v1\"")]).await;
    assert_eq!(fresh.status(), 304);
    assert_eq!(fresh.headers()["etag"], "\"v1\"");

    let stale = server.get(&token, &[("if-none-match", "\"v0\"")]).await;
    assert_eq!(stale.status(), 200);

    let wildcard = server.get(&token, &[("if-none-match", "*")]).await;
    assert_eq!(wildcard.status(), 304);
}

#[tokio::test]
async fn head_reports_metadata_without_a_body() {
    let server = TestServer::start(32, 1 << 20, None).await;
    let token = create_token(&server, Some(b"vault one")).await;

    let response = server
        .client
        .head(server.url("/v1/vault"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("head");
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["etag"], "\"v1\"");
    assert_eq!(response.headers()["x-maine-size"], "9");
}

#[tokio::test]
async fn a_stale_write_conflicts_and_reports_the_current_state() {
    let server = TestServer::start(32, 1 << 20, None).await;
    let token = create_token(&server, Some(b"one")).await;

    let advanced = server.put(&token, b"two", &[("if-match", "\"v1\"")]).await;
    assert_eq!(advanced.status(), 200);
    let json: Value = advanced.json().await.expect("put JSON");
    assert_eq!(json["version"], 2);
    assert_eq!(json["etag"], "\"v2\"");

    let stale = server
        .put(&token, b"three", &[("if-match", "\"v1\"")])
        .await;
    assert_eq!(stale.status(), 409);
    assert_eq!(stale.headers()["x-maine-version"], "2");
    let json: Value = stale.json().await.expect("conflict JSON");
    assert_eq!(json["version"], 2);
    assert_eq!(json["etag"], "\"v2\"");

    // The losing write changed nothing.
    let current = server.get(&token, &[]).await;
    assert_eq!(body_text(current).await, "two");
}

#[tokio::test]
async fn updating_an_existing_vault_requires_a_precondition() {
    let server = TestServer::start(32, 1 << 20, None).await;
    let token = create_token(&server, Some(b"one")).await;

    let response = server.put(&token, b"blind", &[]).await;
    assert_eq!(response.status(), 428);
    assert_eq!(response.headers()["x-maine-version"], "1");
}

#[tokio::test]
async fn the_first_write_to_an_empty_slot_needs_no_precondition() {
    let server = TestServer::start(32, 1 << 20, None).await;
    let token = create_token(&server, None).await;

    let response = server.put(&token, b"first", &[]).await;
    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.expect("put JSON");
    assert_eq!(json["version"], 1);

    // ...but only the first one.
    let second = server.put(&token, b"second", &[]).await;
    assert_eq!(second.status(), 428);
}

#[tokio::test]
async fn if_none_match_star_refuses_to_overwrite_an_existing_vault() {
    let server = TestServer::start(32, 1 << 20, None).await;
    let token = create_token(&server, Some(b"one")).await;

    let response = server.put(&token, b"two", &[("if-none-match", "*")]).await;
    assert_eq!(response.status(), 412);
    assert_eq!(response.headers()["x-maine-version"], "1");
}

#[tokio::test]
async fn missing_and_unknown_tokens_are_rejected() {
    let server = TestServer::start(32, 1 << 20, None).await;

    let anonymous = server
        .client
        .get(server.url("/v1/vault"))
        .send()
        .await
        .expect("anonymous");
    assert_eq!(anonymous.status(), 401);
    assert_eq!(anonymous.headers()["www-authenticate"], "Bearer");

    let unknown_token = format!("maine-{}", "A".repeat(52));
    let response = server.get(&unknown_token, &[]).await;
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn tokens_are_tolerated_in_any_display_form() {
    let server = TestServer::start(32, 1 << 20, None).await;
    let token = create_token(&server, Some(b"one")).await;

    let sloppy = token.replace('-', " ").to_lowercase();
    let response = server
        .client
        .get(server.url("/v1/vault"))
        .header("authorization", format!("bearer   {sloppy}"))
        .send()
        .await
        .expect("sloppy request");
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn rotation_invalidates_the_previous_token() {
    let server = TestServer::start(32, 1 << 20, None).await;
    let old_token = create_token(&server, Some(b"one")).await;

    let response = server
        .client
        .post(server.url("/v1/vault/rotate"))
        .bearer_auth(&old_token)
        .send()
        .await
        .expect("rotate");
    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.expect("rotate JSON");
    let new_token = json["token"].as_str().expect("token").to_owned();
    assert_ne!(new_token, old_token);

    assert_eq!(server.get(&old_token, &[]).await.status(), 401);
    let response = server.get(&new_token, &[]).await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_text(response).await, "one");
}

#[tokio::test]
async fn delete_wipes_the_slot_and_its_history() {
    let server = TestServer::start(32, 1 << 20, None).await;
    let token = create_token(&server, Some(b"one")).await;

    let response = server
        .client
        .delete(server.url("/v1/vault"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("delete");
    assert_eq!(response.status(), 204);

    assert_eq!(server.get(&token, &[]).await.status(), 401);
}

#[tokio::test]
async fn history_keeps_the_last_versions_and_prunes_older_ones() {
    let server = TestServer::start(2, 1 << 20, None).await;
    let token = create_token(&server, Some(b"one")).await;

    for (body, etag) in [
        (&b"two"[..], "\"v1\""),
        (&b"three"[..], "\"v2\""),
        (&b"four"[..], "\"v3\""),
    ] {
        let response = server.put(&token, body, &[("if-match", etag)]).await;
        assert_eq!(response.status(), 200);
    }

    let listing: Value = server
        .versions(&token)
        .await
        .json()
        .await
        .expect("versions");
    let numbers: Vec<i64> = listing["versions"]
        .as_array()
        .expect("versions array")
        .iter()
        .map(|entry| entry["version"].as_i64().expect("version"))
        .collect();
    assert_eq!(numbers, vec![4, 3]);
    assert_eq!(
        listing["versions"][0]["sha256"],
        hash::hex(&hash::sha256(b"four"))
    );

    // Pruned versions are gone, kept ones are fetchable byte-for-byte.
    for (number, expected) in [(2, None), (3, Some("three"))] {
        let response = server
            .client
            .get(server.url(&format!("/v1/vault/versions/{number}")))
            .bearer_auth(&token)
            .send()
            .await
            .expect("version fetch");
        match expected {
            Some(text) => {
                assert_eq!(response.status(), 200);
                assert_eq!(body_text(response).await, text);
            }
            None => assert_eq!(response.status(), 404),
        }
    }
}

#[tokio::test]
async fn oversized_bodies_are_rejected_without_storing_anything() {
    let server = TestServer::start(32, 64, None).await;
    let big = vec![b'x'; 100];

    let response = server.create(Some(&big), None).await;
    assert_eq!(response.status(), 413);

    let token = create_token(&server, None).await;
    let response = server.put(&token, &big, &[("if-none-match", "*")]).await;
    assert_eq!(response.status(), 413);
    assert_eq!(server.get(&token, &[]).await.status(), 204);
}

#[tokio::test]
async fn the_create_secret_gates_slot_creation_only() {
    let server = TestServer::start(32, 1 << 20, Some("open-sesame")).await;

    assert_eq!(server.create(None, None).await.status(), 403);
    assert_eq!(server.create(None, Some("wrong")).await.status(), 403);

    let response = server.create(Some(b"one"), Some("open-sesame")).await;
    assert_eq!(response.status(), 201);
    let json: Value = response.json().await.expect("create JSON");
    let token = json["token"].as_str().expect("token");

    // Existing slots keep working without the secret.
    assert_eq!(server.get(token, &[]).await.status(), 200);
}

#[tokio::test]
async fn concurrent_writers_leave_exactly_one_winner() {
    let server = TestServer::start(32, 1 << 20, None).await;
    let token = create_token(&server, Some(b"base")).await;

    let (first, second) = tokio::join!(
        server.put(&token, b"first", &[("if-match", "\"v1\"")]),
        server.put(&token, b"second", &[("if-match", "\"v1\"")]),
    );
    let mut statuses = [first.status().as_u16(), second.status().as_u16()];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 409]);

    let current = server.get(&token, &[]).await;
    let stored = body_text(current).await;
    assert!(stored == "first" || stored == "second");
}
