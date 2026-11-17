use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::Instant;

#[derive(Debug, Clone)]
struct Recorded {
    request_line: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Recorded {
    fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// A scripted HTTP/1.1 server on a loopback port: answers each connection
/// with the next canned response and records what it was sent.
struct MiniServer {
    url: String,
    requests: Arc<Mutex<Vec<Recorded>>>,
    stop: Arc<AtomicBool>,
    port: u16,
    thread: Option<JoinHandle<()>>,
}

fn response(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 {status}\r\nConnection: close\r\n").into_bytes();
    for (name, value) in headers {
        out.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    out.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
    out.extend_from_slice(body);
    out
}

fn json(status: &str, headers: &[(&str, &str)], body: &str) -> Vec<u8> {
    let mut all = vec![("Content-Type", "application/json")];
    all.extend_from_slice(headers);
    response(status, &all, body.as_bytes())
}

fn read_request(stream: &mut TcpStream) -> Option<Recorded> {
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        data.extend_from_slice(&buf[..n]);
    };
    let head = String::from_utf8_lossy(&data[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let length = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = data[header_end + 4..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
    }
    Some(Recorded {
        request_line,
        headers,
        body,
    })
}

impl MiniServer {
    fn start(responses: Vec<Vec<u8>>) -> Self {
        Self::spawn(responses, None)
    }

    /// Accepts connections and never answers within `hold`.
    fn hanging(hold: Duration) -> Self {
        Self::spawn(vec![Vec::new(); 8], Some(hold))
    }

    fn spawn(responses: Vec<Vec<u8>>, hold: Option<Duration>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (log, flag) = (requests.clone(), stop.clone());
        let thread = thread::spawn(move || {
            for canned in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                if flag.load(Ordering::SeqCst) {
                    return;
                }
                let Some(request) = read_request(&mut stream) else {
                    continue;
                };
                log.lock().unwrap().push(request);
                if let Some(hold) = hold {
                    thread::sleep(hold);
                    continue;
                }
                let _ = stream.write_all(&canned);
                let _ = stream.flush();
            }
        });
        Self {
            url: format!("http://127.0.0.1:{port}"),
            requests,
            stop,
            port,
            thread: Some(thread),
        }
    }

    fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for MiniServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake a blocked accept so the thread can exit.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn transport() -> HttpTransport {
    HttpTransport::with_request_timeout(Duration::from_secs(5))
}

#[test]
fn create_posts_the_body_and_returns_the_token_once() {
    let server = MiniServer::start(vec![json(
        "201 Created",
        &[],
        r#"{"token":"maine-AAAA-BBBB","version":1,"etag":"\"v1\"","size":3,"sha256":"abc"}"#,
    )]);

    let (token, state) = transport()
        .create_vault(&server.url, Some("the-secret"), b"xyz")
        .unwrap();

    assert_eq!(token, "maine-AAAA-BBBB");
    assert_eq!(state, SlotState::with(1, 3, "abc"));
    let request = &server.requests()[0];
    assert_eq!(request.request_line, "POST /v1/vaults HTTP/1.1");
    assert_eq!(request.body, b"xyz");
    assert_eq!(request.header("x-maine-create-secret"), Some("the-secret"));
    assert_eq!(
        request.header("content-type"),
        Some("application/octet-stream")
    );
    assert_eq!(request.header("user-agent"), Some("Maine-Sync"));
    assert_eq!(request.header("authorization"), None);
}

#[test]
fn create_without_a_secret_sends_no_secret_header() {
    let server = MiniServer::start(vec![json(
        "201 Created",
        &[],
        r#"{"token":"maine-T","version":0,"size":0}"#,
    )]);

    let (_, state) = transport()
        .create_vault(&server.url, Some(""), b"")
        .unwrap();

    assert_eq!(state, SlotState::new(0));
    assert_eq!(server.requests()[0].header("x-maine-create-secret"), None);
}

#[test]
fn put_sends_if_match_or_if_none_match_and_the_bearer_token() {
    let ok = json(
        "200 OK",
        &[],
        r#"{"version":4,"etag":"\"v4\"","size":2,"sha256":"def"}"#,
    );
    let server = MiniServer::start(vec![ok.clone(), ok]);
    let transport = transport();

    let state = transport
        .put_vault(&server.url, "maine-TOK", b"ab", Some(3), None)
        .unwrap();
    transport
        .put_vault(
            &server.url,
            "maine-TOK",
            b"ab",
            None,
            Some(Duration::from_secs(3)),
        )
        .unwrap();

    assert_eq!(state, SlotState::with(4, 2, "def"));
    let requests = server.requests();
    assert_eq!(requests[0].request_line, "PUT /v1/vault HTTP/1.1");
    assert_eq!(requests[0].header("if-match"), Some("\"v3\""));
    assert_eq!(requests[0].header("if-none-match"), None);
    assert_eq!(
        requests[0].header("authorization"),
        Some("Bearer maine-TOK")
    );
    assert_eq!(requests[0].body, b"ab");
    assert_eq!(requests[1].header("if-none-match"), Some("*"));
    assert_eq!(requests[1].header("if-match"), None);
}

#[test]
fn fetch_reads_every_documented_outcome() {
    let headers = [
        ("X-Maine-Version", "4"),
        ("X-Maine-Size", "5"),
        ("X-Maine-SHA256", "feed"),
        ("X-Maine-Updated-At", "1730000000"),
    ];
    let server = MiniServer::start(vec![
        response("200 OK", &headers, b"bytes"),
        response("304 Not Modified", &[("X-Maine-Version", "4")], b""),
        response("204 No Content", &[("X-Maine-Version", "0")], b""),
        response("200 OK", &headers, b""),
    ]);
    let transport = transport();

    let blob = transport.fetch_vault(&server.url, "t", None).unwrap();
    let not_modified = transport.fetch_vault(&server.url, "t", Some(4)).unwrap();
    let empty = transport.fetch_vault(&server.url, "t", Some(0)).unwrap();
    let head = transport.head_vault(&server.url, "t", Some(2)).unwrap();

    let state = SlotState {
        version: 4,
        size: 5,
        sha256: Some("feed".into()),
        updated_at: Some(1_730_000_000),
    };
    assert_eq!(blob, FetchResult::Blob(b"bytes".to_vec(), state.clone()));
    assert_eq!(not_modified, FetchResult::NotModified(SlotState::new(4)));
    assert_eq!(empty, FetchResult::Empty(SlotState::new(0)));
    assert_eq!(head, FetchResult::Current(state));

    let requests = server.requests();
    assert_eq!(requests[0].request_line, "GET /v1/vault HTTP/1.1");
    assert_eq!(requests[0].header("if-none-match"), None);
    assert_eq!(requests[1].header("if-none-match"), Some("\"v4\""));
    // Version 0 means "never synced": no conditional header.
    assert_eq!(requests[2].header("if-none-match"), None);
    assert_eq!(requests[3].request_line, "HEAD /v1/vault HTTP/1.1");
    assert_eq!(requests[3].header("if-none-match"), Some("\"v2\""));
}

#[test]
fn status_codes_map_to_the_documented_errors() {
    let server = MiniServer::start(vec![
        json(
            "401 Unauthorized",
            &[("WWW-Authenticate", "Bearer")],
            r#"{"error":"nope"}"#,
        ),
        json(
            "403 Forbidden",
            &[],
            r#"{"error":"a create secret is required"}"#,
        ),
        json("404 Not Found", &[], r#"{"error":"no such version"}"#),
        json(
            "409 Conflict",
            &[("X-Maine-Version", "9")],
            r#"{"error":"stale","version":9}"#,
        ),
        json(
            "412 Precondition Failed",
            &[("X-Maine-Version", "2")],
            r#"{"error":"exists"}"#,
        ),
        json("413 Payload Too Large", &[], r#"{"error":"too big"}"#),
        json(
            "428 Precondition Required",
            &[("X-Maine-Version", "5")],
            r#"{"error":"need one"}"#,
        ),
        json(
            "500 Internal Server Error",
            &[],
            r#"{"error":"database is locked"}"#,
        ),
        response("502 Bad Gateway", &[], b"<html>proxy</html>"),
    ]);
    let transport = transport();
    let put = |expected| transport.put_vault(&server.url, "t", b"x", expected, None);

    assert_eq!(put(Some(1)).unwrap_err(), SyncError::Unauthorized);
    assert_eq!(
        put(Some(1)).unwrap_err(),
        SyncError::Forbidden("a create secret is required".into())
    );
    assert_eq!(
        put(Some(1)).unwrap_err(),
        SyncError::NotFound("no such version".into())
    );
    assert_eq!(
        put(Some(1)).unwrap_err(),
        SyncError::Conflict { current_version: 9 }
    );
    assert_eq!(
        put(None).unwrap_err(),
        SyncError::Conflict { current_version: 2 }
    );
    assert_eq!(put(Some(1)).unwrap_err(), SyncError::TooLarge);
    assert_eq!(
        put(None).unwrap_err(),
        SyncError::PreconditionRequired { current_version: 5 }
    );
    assert_eq!(
        put(Some(1)).unwrap_err(),
        SyncError::Server {
            status: 500,
            message: "database is locked".into()
        }
    );
    assert_eq!(
        put(Some(1)).unwrap_err(),
        SyncError::Server {
            status: 502,
            message: "The server answered with status 502.".into()
        }
    );
}

#[test]
fn a_redirect_is_never_followed() {
    let other = MiniServer::start(vec![json("200 OK", &[], r#"{"token":"x","version":1}"#)]);
    let server = MiniServer::start(vec![response(
        "302 Found",
        &[("Location", &format!("{}/v1/vault", other.url))],
        b"",
    )]);

    let error = transport()
        .fetch_vault(&server.url, "maine-SECRET", None)
        .unwrap_err();

    assert!(
        matches!(error, SyncError::Server { status: 302, .. }),
        "{error:?}"
    );
    assert_eq!(server.requests().len(), 1);
    // The other host never saw the token (or anything else).
    assert!(other.requests().is_empty());
}

#[test]
fn a_base_path_is_kept_in_every_request() {
    let server = MiniServer::start(vec![json("200 OK", &[], r#"{"token":"maine-NEW"}"#)]);
    let base = format!("{}/maine", server.url);

    assert_eq!(transport().rotate_token(&base, "t").unwrap(), "maine-NEW");

    let request = &server.requests()[0];
    assert_eq!(request.request_line, "POST /maine/v1/vault/rotate HTTP/1.1");
    assert_eq!(request.header("authorization"), Some("Bearer t"));
}

#[test]
fn delete_accepts_204_and_200() {
    let server = MiniServer::start(vec![
        response("204 No Content", &[], b""),
        response("200 OK", &[], b""),
        json("401 Unauthorized", &[], r#"{"error":"x"}"#),
    ]);
    let transport = transport();

    transport.delete_vault(&server.url, "t").unwrap();
    transport.delete_vault(&server.url, "t").unwrap();
    assert_eq!(
        transport.delete_vault(&server.url, "t").unwrap_err(),
        SyncError::Unauthorized
    );
    assert_eq!(
        server.requests()[0].request_line,
        "DELETE /v1/vault HTTP/1.1"
    );
}

#[test]
fn versions_list_and_fetch() {
    let server = MiniServer::start(vec![
        json(
            "200 OK",
            &[],
            r#"{"versions":[{"version":4,"size":10,"sha256":"aa","createdAt":1730000000,"extra":1},{"version":3,"size":9,"sha256":"bb","createdAt":1729990000}]}"#,
        ),
        response(
            "200 OK",
            &[
                ("X-Maine-Version", "3"),
                ("X-Maine-Size", "3"),
                ("X-Maine-SHA256", "bb"),
            ],
            b"old",
        ),
        json(
            "404 Not Found",
            &[],
            r#"{"error":"version 1 is not stored"}"#,
        ),
    ]);
    let transport = transport();

    let versions = transport.list_versions(&server.url, "t").unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].version, 4);
    assert_eq!(versions[1].created_at, 1_729_990_000);

    let (bytes, state) = transport.fetch_version(&server.url, "t", 3).unwrap();
    assert_eq!(bytes, b"old");
    assert_eq!(state.sha256.as_deref(), Some("bb"));

    assert_eq!(
        transport.fetch_version(&server.url, "t", 1).unwrap_err(),
        SyncError::NotFound("version 1 is not stored".into())
    );
    let requests = server.requests();
    assert_eq!(requests[0].request_line, "GET /v1/vault/versions HTTP/1.1");
    assert_eq!(
        requests[1].request_line,
        "GET /v1/vault/versions/3 HTTP/1.1"
    );
}

#[test]
fn health_needs_no_token() {
    let server = MiniServer::start(vec![json("200 OK", &[], r#"{"status":"ok"}"#)]);

    transport().health(&server.url).unwrap();

    let request = &server.requests()[0];
    assert_eq!(request.request_line, "GET /v1/health HTTP/1.1");
    assert_eq!(request.header("authorization"), None);
}

#[test]
fn malformed_json_is_malformed_not_a_panic() {
    let server = MiniServer::start(vec![
        json("201 Created", &[], "this is not json"),
        json("200 OK", &[], r#"{"version":"two"}"#),
        json("200 OK", &[], "{}"),
    ]);
    let transport = transport();

    assert!(matches!(
        transport.create_vault(&server.url, None, b"x").unwrap_err(),
        SyncError::Malformed(_)
    ));
    assert!(matches!(
        transport
            .put_vault(&server.url, "t", b"x", Some(1), None)
            .unwrap_err(),
        SyncError::Malformed(_)
    ));
    assert!(matches!(
        transport.rotate_token(&server.url, "t").unwrap_err(),
        SyncError::Malformed(_)
    ));
}

#[test]
fn an_unexpected_success_status_is_a_server_error() {
    let server = MiniServer::start(vec![json("200 OK", &[], r#"{"token":"x","version":1}"#)]);

    let error = transport()
        .create_vault(&server.url, None, b"x")
        .unwrap_err();

    assert!(
        matches!(error, SyncError::Server { status: 200, .. }),
        "{error:?}"
    );
}

#[test]
fn a_closed_port_is_offline() {
    // Bind then drop to find a port nothing listens on.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();

    let error = transport()
        .fetch_vault(&format!("http://127.0.0.1:{port}"), "t", None)
        .unwrap_err();

    assert_eq!(
        error,
        SyncError::Offline("The sync server could not be reached.".into())
    );
}

#[test]
fn an_unreachable_name_is_offline_and_hides_the_host() {
    let error = transport()
        .fetch_vault("http://sync-host.invalid", "t", None)
        .unwrap_err();

    match error {
        SyncError::Offline(message) => assert!(!message.contains("invalid"), "{message}"),
        other => panic!("expected offline, got {other:?}"),
    }
}

#[test]
fn a_silent_server_times_out_and_the_per_call_timeout_is_honoured() {
    let server = MiniServer::hanging(Duration::from_secs(3));
    let transport = HttpTransport::with_request_timeout(Duration::from_millis(400));

    let started = Instant::now();
    let error = transport.fetch_vault(&server.url, "t", None).unwrap_err();
    assert_eq!(
        error,
        SyncError::Offline("The sync server took too long to answer.".into())
    );
    assert!(started.elapsed() < Duration::from_secs(2));

    // A longer default, but a short bound on this one push.
    let patient = HttpTransport::with_request_timeout(Duration::from_secs(30));
    let started = Instant::now();
    let error = patient
        .put_vault(
            &server.url,
            "t",
            b"x",
            Some(1),
            Some(Duration::from_millis(300)),
        )
        .unwrap_err();
    assert!(matches!(error, SyncError::Offline(_)), "{error:?}");
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn a_bad_address_is_a_malformed_error() {
    let error = transport().fetch_vault("not a url", "t", None).unwrap_err();
    assert!(matches!(error, SyncError::Malformed(_)), "{error:?}");
}

#[test]
fn a_token_that_cannot_be_a_header_value_is_refused_without_echoing_it() {
    let server = MiniServer::start(vec![]);

    let error = transport()
        .fetch_vault(&server.url, "bad\ntoken", None)
        .unwrap_err();

    match error {
        SyncError::Malformed(message) => assert!(!message.contains("bad"), "{message}"),
        other => panic!("expected malformed, got {other:?}"),
    }
    assert!(server.requests().is_empty());
}
