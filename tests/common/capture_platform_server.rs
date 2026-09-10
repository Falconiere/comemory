#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    dead_code
)]
//! Loopback stand-in for `GET /v1/capture/sources` and `POST /v1/sessions`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

const MAX_BODY: usize = 1_000_000;

/// Mutable capture-platform fixture state.
#[derive(Debug, Clone)]
pub struct CapturePlatformState {
    /// Consent rows returned by GET sources.
    pub sources: Value,
    /// Last POST /v1/sessions body.
    pub last_receipt: Option<String>,
    /// `created` flag in the POST response.
    pub created: bool,
    /// Optional error code to return on POST.
    pub post_error: Option<(u16, String, String)>,
}

impl Default for CapturePlatformState {
    fn default() -> Self {
        Self {
            sources: json!([
                {"source":"claude-code","enabled":true,"updatedAt":"2026-09-10T00:00:00.000Z"},
                {"source":"cursor","enabled":false,"updatedAt":null},
                {"source":"codex","enabled":false,"updatedAt":null},
                {"source":"ci","enabled":false,"updatedAt":null},
                {"source":"chat-export","enabled":false,"updatedAt":null}
            ]),
            last_receipt: None,
            created: true,
            post_error: None,
        }
    }
}

/// One recorded request.
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    /// HTTP method.
    pub method: String,
    /// Path without query.
    pub path: String,
    /// Authorization header.
    pub authorization: String,
    /// Workspace header, if any.
    pub workspace_header: Option<String>,
}

/// Running loopback server.
pub struct CapturePlatformServer {
    /// Base URL (`http://127.0.0.1:port`).
    pub base_url: String,
    state: Arc<Mutex<CapturePlatformState>>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl CapturePlatformServer {
    /// Bind and serve on a background thread.
    pub fn spawn(state: CapturePlatformState) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let state = Arc::new(Mutex::new(state));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let state_t = Arc::clone(&state);
        let req_t = Arc::clone(&requests);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = handle(stream, &state_t, &req_t);
            }
        });
        Self {
            base_url: format!("http://{addr}"),
            state,
            requests,
        }
    }

    /// Snapshot of recorded requests.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().expect("lock").clone()
    }

    /// Last posted receipt body.
    pub fn last_receipt(&self) -> Option<String> {
        self.state.lock().expect("lock").last_receipt.clone()
    }
}

fn handle(
    mut stream: TcpStream,
    state: &Arc<Mutex<CapturePlatformState>>,
    requests: &Arc<Mutex<Vec<RecordedRequest>>>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let path = target.split('?').next().unwrap_or("").to_string();

    let mut content_length = 0usize;
    let mut authorization = String::new();
    let mut workspace_header = None;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            content_length = rest.trim().parse().unwrap_or(0);
        } else if let Some(rest) = line.split_once(':') {
            let (name, value) = rest;
            let value = value.trim().trim_end_matches(['\r', '\n']).to_string();
            if name.eq_ignore_ascii_case("authorization") {
                authorization = value;
            } else if name.eq_ignore_ascii_case("x-comemory-workspace") {
                workspace_header = Some(value);
            }
        }
    }
    let mut body = vec![0u8; content_length.min(MAX_BODY)];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }
    requests.lock().expect("lock").push(RecordedRequest {
        method: method.clone(),
        path: path.clone(),
        authorization,
        workspace_header,
    });

    let (status, payload) = match (method.as_str(), path.as_str()) {
        ("GET", "/v1/capture/sources") => {
            let sources = state.lock().expect("lock").sources.clone();
            (200, json!({"ok":true,"data":{"sources":sources},"meta":{}}))
        }
        ("POST", "/v1/sessions") => {
            let mut st = state.lock().expect("lock");
            if let Some((code, err, msg)) = st.post_error.clone() {
                (
                    code,
                    json!({"ok":false,"error":{"code":err,"message":msg},"meta":{}}),
                )
            } else {
                st.last_receipt = Some(String::from_utf8_lossy(&body).into_owned());
                let created = st.created;
                (
                    200,
                    json!({
                        "ok": true,
                        "data": {
                            "session": {
                                "id": "sess-1",
                                "externalId": "ext",
                            },
                            "created": created
                        },
                        "meta": {}
                    }),
                )
            }
        }
        _ => (
            404,
            json!({"ok":false,"error":{"code":"not_found","message":path},"meta":{}}),
        ),
    };
    let body = serde_json::to_vec(&payload).unwrap();
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Error",
    };
    let resp = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(resp.as_bytes())?;
    stream.write_all(&body)?;
    Ok(())
}
