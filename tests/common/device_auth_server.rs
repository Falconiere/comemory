#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Loopback stand-in for the platform device-auth + mint flow.
//!
//! Speaks real HTTP over a real socket so `comemory auth` still shells out
//! to curl/wget. Covers `/auth/device/code`, `/auth/device/token`,
//! `/v1/device/mint-device-key`, and `/v1/workspaces`. Each consuming
//! binary `#[path]`-includes this file directly.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// Behaviour knobs for one fixture instance.
#[derive(Debug, Clone)]
pub struct DeviceAuthConfig {
    /// How many token polls return `authorization_pending` before success.
    pub pending_polls: u32,
    /// Reject every `/auth/device/code` with `invalid_client`.
    pub reject_client: bool,
    /// Every token poll returns `expired_token`.
    pub expire_token: bool,
    /// Device `access_token` returned on success.
    pub access_token: String,
    /// Minted device key secret (`cmk_` + 64 hex).
    pub secret: String,
    /// Display prefix for the minted key.
    pub key_prefix: String,
    /// Personal workspace UUID returned by the mint.
    pub workspace_id: String,
    /// Workspace display name.
    pub workspace_name: String,
    /// `apiUrl` the mint echoes back (blank → the CLI keeps the dialed URL).
    pub mint_api_url: String,
    /// Status `GET /v1/workspaces` answers with (200 → the normal list).
    pub workspaces_status: u16,
}

impl Default for DeviceAuthConfig {
    fn default() -> Self {
        Self {
            pending_polls: 0,
            reject_client: false,
            expire_token: false,
            access_token: "dev-access-token".into(),
            secret: "cmk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            key_prefix: "cmk_aaaa".into(),
            workspace_id: "11111111-2222-3333-4444-555555555555".into(),
            workspace_name: "Fixture Workspace".into(),
            mint_api_url: String::new(),
            workspaces_status: 200,
        }
    }
}

/// A running fixture server. Lives until the process exits.
pub struct DeviceAuthServer {
    /// `http://127.0.0.1:<port>` — pass as `--api-url` / `COMEMORY_API`.
    pub base: String,
    /// Shared config (tests may inspect minted values).
    pub config: Arc<DeviceAuthConfig>,
}

struct Shared {
    config: DeviceAuthConfig,
    /// device_code → remaining pending polls (initialized from config).
    codes: Mutex<HashMap<String, u32>>,
    next_code: AtomicU32,
}

impl DeviceAuthServer {
    /// Bind loopback and serve with `config`.
    pub fn start(config: DeviceAuthConfig) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        let base = format!("http://127.0.0.1:{port}");
        let shared = Arc::new(Shared {
            config: config.clone(),
            codes: Mutex::new(HashMap::new()),
            next_code: AtomicU32::new(1),
        });
        let thread_shared = Arc::clone(&shared);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = handle(stream, &thread_shared);
            }
        });
        Self {
            base,
            config: Arc::new(config),
        }
    }

    /// Convenience: happy-path defaults.
    pub fn start_default() -> Self {
        Self::start(DeviceAuthConfig::default())
    }
}

/// Whether curl/wget are on PATH (same bar as `release_server`).
pub fn tooling_present() -> bool {
    ["curl", "wget"].iter().any(|tool| {
        std::process::Command::new(tool)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

fn handle(mut stream: TcpStream, shared: &Shared) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();
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
    let (status, content_type, resp) = route(&method, &path, &body_str, &authorization, shared);
    let head = format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n",
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
    body: &str,
    authorization: &str,
    shared: &Shared,
) -> (&'static str, &'static str, String) {
    match (method, path) {
        ("POST", "/auth/device/code") => device_code(body, shared),
        ("POST", "/auth/device/token") => device_token(body, shared),
        ("POST", "/v1/device/mint-device-key") => mint(body, authorization, shared),
        ("GET", "/v1/workspaces") => workspaces_list(authorization, shared),
        _ => (
            "404 Not Found",
            "application/json",
            r#"{"error":"not_found"}"#.into(),
        ),
    }
}

fn device_code(body: &str, shared: &Shared) -> (&'static str, &'static str, String) {
    if shared.config.reject_client {
        return (
            "400 Bad Request",
            "application/json",
            r#"{"error":"invalid_client"}"#.into(),
        );
    }
    let client_id = json_str(body, "client_id").unwrap_or_default();
    if client_id != "comemory-cli" {
        return (
            "400 Bad Request",
            "application/json",
            r#"{"error":"invalid_client"}"#.into(),
        );
    }
    let n = shared.next_code.fetch_add(1, Ordering::SeqCst);
    let device_code = format!("dc-{n}");
    let user_code = format!("USER-{n}");
    shared
        .codes
        .lock()
        .expect("codes lock")
        .insert(device_code.clone(), shared.config.pending_polls);
    let body = serde_json::json!({
        "device_code": device_code,
        "user_code": user_code,
        "verification_uri": format!("{}/device", "http://fixture.local"),
        "verification_uri_complete": format!("http://fixture.local/device?user_code={user_code}"),
        "expires_in": 120,
        "interval": 1,
    });
    ("200 OK", "application/json", body.to_string())
}

fn device_token(body: &str, shared: &Shared) -> (&'static str, &'static str, String) {
    if shared.config.expire_token {
        return (
            "400 Bad Request",
            "application/json",
            r#"{"error":"expired_token"}"#.into(),
        );
    }
    let device_code = json_str(body, "device_code").unwrap_or_default();
    let mut codes = shared.codes.lock().expect("codes lock");
    let Some(remaining) = codes.get_mut(&device_code) else {
        return (
            "400 Bad Request",
            "application/json",
            r#"{"error":"invalid_grant"}"#.into(),
        );
    };
    if *remaining > 0 {
        *remaining -= 1;
        return (
            "400 Bad Request",
            "application/json",
            r#"{"error":"authorization_pending"}"#.into(),
        );
    }
    let token = &shared.config.access_token;
    let body = serde_json::json!({
        "access_token": token,
        "token_type": "Bearer",
    });
    ("200 OK", "application/json", body.to_string())
}

fn mint(body: &str, authorization: &str, shared: &Shared) -> (&'static str, &'static str, String) {
    let expected = format!("Bearer {}", shared.config.access_token);
    if authorization != expected {
        return (
            "401 Unauthorized",
            "application/json",
            r#"{"error":"unauthorized"}"#.into(),
        );
    }
    // The platform's zod input rejects a missing/blank deviceName with 400.
    let device_name = json_str(body, "deviceName").unwrap_or_default();
    if device_name.is_empty() {
        return (
            "400 Bad Request",
            "application/json",
            r#"{"error":"deviceName required"}"#.into(),
        );
    }
    let body = serde_json::json!({
        "secret": shared.config.secret,
        "keyPrefix": shared.config.key_prefix,
        "personalWorkspaceId": shared.config.workspace_id,
        "apiUrl": shared.config.mint_api_url,
    });
    ("201 Created", "application/json", body.to_string())
}

fn workspaces_list(authorization: &str, shared: &Shared) -> (&'static str, &'static str, String) {
    let expected = format!("Bearer {}", shared.config.secret);
    if authorization != expected {
        return (
            "401 Unauthorized",
            "application/json",
            r#"{"error":"unauthorized"}"#.into(),
        );
    }
    if shared.config.workspaces_status == 500 {
        return (
            "500 Internal Server Error",
            "application/json",
            r#"{"error":"server_error"}"#.into(),
        );
    }
    let body = serde_json::json!({
        "workspaces": [{
            "workspace": {
                "id": shared.config.workspace_id,
                "name": shared.config.workspace_name,
            }
        }]
    });
    ("200 OK", "application/json", body.to_string())
}

fn json_str(body: &str, key: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v.get(key)?.as_str().map(str::to_string)
}
