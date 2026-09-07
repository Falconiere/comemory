#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Loopback stand-in for the platform sync + device-key surface.
//!
//! Speaks real HTTP so `comemory::sync::client` (reqwest) hits a real socket.
//! Covers device code/token/mint, workspace list, and the four sync routes.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

/// Mutable responses the fixture serves.
#[derive(Debug, Clone)]
pub struct SyncPlatformState {
    /// Device session token returned by `/auth/device/token`.
    pub access_token: String,
    /// Device key secret (`cmk_…`).
    pub secret: String,
    /// Personal workspace id from mint.
    pub personal_workspace_id: String,
    /// Allowlist returned on `GET /v1/sync/status`.
    pub allowlist: Value,
    /// ETag for allowlist cache short-circuit.
    pub allowlist_etag: Option<String>,
    /// When true, status answers 404 (personal / unbound).
    pub status_404: bool,
    /// `{ok:false}` envelope on status (auth/gate errors).
    pub status_error: Option<(String, String)>,
    /// Entries for `GET /v1/sync/changes` (cleared after one successful read when
    /// `consume_changes` is true).
    pub changes: Value,
    /// Head seq reported on changes / import / manifest.
    pub head_seq: i64,
    /// When true, emptying `changes` after a successful pull.
    pub consume_changes: bool,
    /// 256 bucket digests for manifest (empty → 256 empty-SHA placeholders).
    pub buckets: Option<Vec<String>>,
    /// When set, the *next* manifest response uses these buckets once, then
    /// falls back to `buckets` (repair / re-compare path).
    pub buckets_once: Option<Vec<String>>,
    /// Fixed import response `results` array.
    pub import_results: Value,
    /// Workspace list rows for `GET /v1/workspaces`.
    pub workspaces: Value,
}

impl Default for SyncPlatformState {
    fn default() -> Self {
        Self {
            access_token: "dev-access-token".into(),
            secret: "cmk_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            personal_workspace_id: "ws-personal".into(),
            allowlist: json!([{"fullName": "codasignal/foo", "name": "foo"}]),
            allowlist_etag: Some("etag-1".into()),
            status_404: false,
            status_error: None,
            changes: json!([]),
            head_seq: 0,
            consume_changes: true,
            buckets: None,
            buckets_once: None,
            import_results: json!([]),
            workspaces: json!([{
                "workspace": {
                    "id": "ws-personal",
                    "name": "Personal",
                    "createdAt": "2026-09-06T00:00:00.000Z"
                },
                "corpus": {
                    "memoriesEnabled": true,
                    "codeEnabled": true,
                    "docsEnabled": false
                }
            }]),
        }
    }
}

/// Running fixture. Lives until the process exits.
pub struct SyncPlatformServer {
    /// `http://127.0.0.1:<port>`.
    pub base: String,
    /// Shared mutable state tests can tweak between calls.
    pub state: Arc<Mutex<SyncPlatformState>>,
}

impl SyncPlatformServer {
    /// Bind loopback and serve `state`.
    pub fn start(state: SyncPlatformState) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        let base = format!("http://127.0.0.1:{port}");
        let shared = Arc::new(Mutex::new(state));
        let thread_state = Arc::clone(&shared);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = handle(stream, &thread_state);
            }
        });
        Self {
            base,
            state: shared,
        }
    }

    /// Happy-path defaults.
    pub fn start_default() -> Self {
        Self::start(SyncPlatformState::default())
    }

    /// Snapshot current state.
    pub fn snapshot(&self) -> SyncPlatformState {
        self.state.lock().expect("state").clone()
    }

    /// Replace mutable state mid-test.
    pub fn update<F: FnOnce(&mut SyncPlatformState)>(&self, f: F) {
        f(&mut self.state.lock().expect("state"));
    }
}

/// Empty-bucket digests matching an empty local corpus (`SHA-256("")`).
pub fn empty_manifest_buckets() -> Vec<String> {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(b"");
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    vec![hex; 256]
}

fn handle(mut stream: TcpStream, state: &Arc<Mutex<SyncPlatformState>>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path_q = parts.next().unwrap_or("/").to_string();
    let (path, query) = match path_q.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (path_q, String::new()),
    };
    let mut content_length = 0usize;
    let mut authorization = String::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header == "\r\n" || header == "\n" {
            break;
        }
        let lower = header.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            content_length = rest.trim().parse().unwrap_or(0);
        }
        if lower.starts_with("authorization:") {
            authorization = header
                .split_once(':')
                .map(|(_, v)| v.trim().to_string())
                .unwrap_or_default();
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }
    let body_str = String::from_utf8_lossy(&body);
    let (status, resp) = route(&method, &path, &query, &body_str, &authorization, state);
    let head = format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        resp.len()
    );
    stream.write_all(head.as_bytes())?;
    if method != "HEAD" {
        stream.write_all(resp.as_bytes())?;
    }
    stream.flush()
}

fn route(
    method: &str,
    path: &str,
    query: &str,
    body: &str,
    authorization: &str,
    state: &Arc<Mutex<SyncPlatformState>>,
) -> (&'static str, String) {
    let mut st = state.lock().expect("state");
    match (method, path) {
        ("POST", "/auth/device/code") => (
            "200 OK",
            json!({
                "device_code": "dc-1",
                "user_code": "USER-1",
                "verification_uri": "http://fixture.local/device",
                "expires_in": 120,
                "interval": 1,
            })
            .to_string(),
        ),
        ("POST", "/auth/device/token") => (
            "200 OK",
            json!({
                "access_token": st.access_token,
                "token_type": "Bearer",
            })
            .to_string(),
        ),
        ("POST", "/v1/device/mint-device-key") => {
            let expected = format!("Bearer {}", st.access_token);
            if authorization != expected {
                return (
                    "401 Unauthorized",
                    json!({"error":"unauthorized"}).to_string(),
                );
            }
            (
                "200 OK",
                json!({
                    "secret": st.secret,
                    "keyPrefix": "cmk_bbbb",
                    "personalWorkspaceId": st.personal_workspace_id,
                    "apiUrl": "http://fixture.local",
                    "email": "dev@example.com",
                })
                .to_string(),
            )
        }
        ("GET", "/v1/workspaces") => {
            if !auth_ok(authorization, &st.secret) {
                return (
                    "401 Unauthorized",
                    json!({"error":"unauthorized"}).to_string(),
                );
            }
            (
                "200 OK",
                json!({ "workspaces": st.workspaces.clone() }).to_string(),
            )
        }
        ("GET", "/v1/sync/status") => sync_status(&mut st, authorization),
        ("GET", "/v1/sync/changes") => sync_changes(&mut st, authorization, query),
        ("GET", "/v1/sync/manifest") => sync_manifest(&mut st, authorization),
        ("POST", "/v1/sync/import") => sync_import(&st, authorization, body),
        _ => ("404 Not Found", json!({"error":"not_found"}).to_string()),
    }
}

fn auth_ok(authorization: &str, secret: &str) -> bool {
    authorization == format!("Bearer {secret}")
}

fn envelope_ok(data: Value) -> String {
    json!({"ok": true, "data": data, "meta": {"command": "platform"}}).to_string()
}

fn envelope_err(code: &str, message: &str) -> String {
    json!({
        "ok": false,
        "error": {"code": code, "message": message},
        "meta": {"command": "platform"}
    })
    .to_string()
}

fn sync_status(st: &mut SyncPlatformState, authorization: &str) -> (&'static str, String) {
    if !auth_ok(authorization, &st.secret) {
        return ("401 Unauthorized", envelope_err("unauthorized", "bad key"));
    }
    if st.status_404 {
        return ("404 Not Found", json!({"error":"not_found"}).to_string());
    }
    if let Some((code, msg)) = &st.status_error {
        return ("200 OK", envelope_err(code, msg));
    }
    (
        "200 OK",
        envelope_ok(json!({
            "head_seq": st.head_seq,
            "devices": [],
            "allowlist": st.allowlist.clone(),
            "allowlist_etag": st.allowlist_etag,
            "personal_sync": false
        })),
    )
}

fn sync_changes(
    st: &mut SyncPlatformState,
    authorization: &str,
    _query: &str,
) -> (&'static str, String) {
    if !auth_ok(authorization, &st.secret) {
        return ("401 Unauthorized", envelope_err("unauthorized", "bad key"));
    }
    let entries = st.changes.clone();
    let next_seq = entries
        .as_array()
        .and_then(|a| a.last())
        .and_then(|e| e.get("seq"))
        .and_then(Value::as_i64);
    if st.consume_changes {
        st.changes = json!([]);
    }
    (
        "200 OK",
        envelope_ok(json!({
            "entries": entries,
            "next_seq": next_seq,
            "head_seq": st.head_seq
        })),
    )
}

fn sync_manifest(st: &mut SyncPlatformState, authorization: &str) -> (&'static str, String) {
    if !auth_ok(authorization, &st.secret) {
        return ("401 Unauthorized", envelope_err("unauthorized", "bad key"));
    }
    let buckets = if let Some(once) = st.buckets_once.take() {
        once
    } else {
        st.buckets.clone().unwrap_or_else(empty_manifest_buckets)
    };
    (
        "200 OK",
        envelope_ok(json!({
            "buckets": buckets,
            "head_seq": st.head_seq
        })),
    )
}

fn sync_import(st: &SyncPlatformState, authorization: &str, _body: &str) -> (&'static str, String) {
    if !auth_ok(authorization, &st.secret) {
        return ("401 Unauthorized", envelope_err("unauthorized", "bad key"));
    }
    (
        "200 OK",
        envelope_ok(json!({
            "results": st.import_results.clone(),
            "head_seq": st.head_seq
        })),
    )
}
