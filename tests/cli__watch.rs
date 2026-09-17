#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

//! `comemory watch` against a real socket: it mints a ticket, connects, and
//! applies what the nudge tells it to pull.
//!
//! The fixture speaks real HTTP and a real WebSocket handshake, so the whole
//! path — ticket route, upgrade, frame, cursored pull, markdown write — runs
//! for real. Nothing is stubbed but the platform itself.

use auth_home::Home;
use sync_platform_server::{SyncPlatformServer, SyncPlatformState};

#[path = "common/auth_home.rs"]
mod auth_home;
#[path = "common/device_auth_server.rs"]
mod device_auth_server;
#[path = "common/sync_platform_server.rs"]
mod sync_platform_server;

/// `curl`/`wget` are what the device-login path shells out to.
fn require_http_tools() -> bool {
    device_auth_server::tooling_present()
}

#[test]
fn watch_pulls_what_the_channel_announces() {
    if !require_http_tools() {
        return;
    }
    let srv = SyncPlatformServer::start(SyncPlatformState::default());

    // A memory only the organization has: the pull the nudge triggers is the
    // only way it can reach this machine.
    let body = "a decision pushed from the other laptop";
    let id = comemory::memory::id::memory_id(body);
    // The same content hash the engine derives: the id is its first 8 hex.
    let content_hash = {
        use sha2::{Digest as _, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(body.trim_end().as_bytes());
        hasher.finalize().iter().fold(String::new(), |mut acc, b| {
            use std::fmt::Write as _;
            let _ = write!(acc, "{b:02x}");
            acc
        })
    };
    srv.update(|st| {
        st.consume_changes = true;
        st.head_seq = 1;
        st.changes = serde_json::json!([{
            "seq": 1,
            "op": "upsert",
            "id": id,
            "content_hash": content_hash,
            "at": "2026-09-14T00:00:00Z",
            "author": "someone-else",
            "record": {
                "frontmatter": {
                    "id": id,
                    "kind": "decision",
                    "repo": "",
                    "created": "2026-09-14T00:00:00Z",
                    "tags": [],
                    "quality": 3,
                    "schema": 1,
                    "content_hash": content_hash,
                },
                "body": body,
            }
        }]);
    });

    let home = Home::new();
    let login = home.run(None, &["auth", "login", "--api-url", &srv.base, "--json"]);
    assert!(
        login.status.success(),
        "login failed: {}",
        String::from_utf8_lossy(&login.stderr)
    );

    let out = home.run(None, &["watch", "--once", "--json"]);
    assert!(
        out.status.success(),
        "watch failed: {} / {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // The upgrade really happened, carrying a ticket the client had to mint.
    let paths = srv.paths();
    assert!(
        paths.iter().any(|p| p == "/v1/ws/ticket"),
        "watch must mint a ticket first, saw: {paths:?}"
    );
    assert!(
        paths.iter().any(|p| p == "/v1/ws"),
        "watch must open the channel, saw: {paths:?}"
    );

    // And the pull it triggered wrote the memory to disk.
    let memories = std::fs::read_dir(home.data_dir().join("memories"))
        .expect("memories dir")
        .filter_map(std::result::Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    // Anchored at both ends: the store names a memory `{id}-{slug}.md`, so a
    // bare `starts_with` would also accept `{id}_something_else`.
    assert!(
        memories.iter().any(|name| {
            std::path::Path::new(name)
                .extension()
                .is_some_and(|ext| ext == "md")
                && name.starts_with(&format!("{id}-"))
        }),
        "the announced memory must be on disk after the triggered pull: {memories:?}"
    );

    // The `--json` contract: one object per event, exactly two keys, the
    // greeting first. Asserted here because `cli::watch::report` is the only
    // thing that renders it and this is the only suite that sees its stdout.
    //
    // Both counts are zero: `auth login` above already ran the initial sync,
    // and the fixture consumes its changes, so the nudge's cursored pull finds
    // nothing left to apply. That is the empty-pull path a duplicate frame
    // takes, and it is the shape — not the arithmetic — this assertion guards.
    let events: Vec<serde_json::Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("each line is one JSON object"))
        .collect();
    assert_eq!(
        events,
        vec![
            serde_json::json!({"event": "connected", "pulled": 0}),
            serde_json::json!({"event": "pulled", "pulled": 0}),
        ],
        "watch --json emitted {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn watch_without_a_login_is_a_usage_error() {
    let home = Home::new();
    let out = home.run(None, &["watch", "--once"]);
    assert!(!out.status.success(), "watch must refuse without auth.json");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("auth login"),
        "the error must point at login: {stderr}"
    );
}
