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

use std::path::PathBuf;
use std::process::Output;

use assert_cmd::cargo::cargo_bin;
use tempfile::TempDir;

#[path = "common/sync_platform_server.rs"]
mod sync_platform_server;

use sync_platform_server::{SyncPlatformServer, SyncPlatformState};

/// A throwaway `$HOME` + data dir, as the auth suite uses.
struct Home {
    root: TempDir,
}

impl Home {
    fn new() -> Self {
        Self {
            root: TempDir::new().unwrap(),
        }
    }

    fn data_dir(&self) -> PathBuf {
        self.root.path().join(".comemory")
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut cmd = std::process::Command::new(cargo_bin("comemory"));
        cmd.env("COMEMORY_DATA_DIR", self.data_dir())
            .env("HOME", self.root.path())
            .env("COMEMORY_SYNC_DAEMON", "0")
            .env_remove("COMEMORY_API")
            .env_remove("COMEMORY_API_KEY")
            .args(args);
        cmd.output().expect("run comemory")
    }
}

/// `curl`/`wget` are what the device-login path shells out to.
fn require_http_tools() -> bool {
    ["curl", "wget"].iter().any(|tool| {
        std::process::Command::new(tool)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
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
    let login = home.run(&["auth", "login", "--api-url", &srv.base, "--json"]);
    assert!(
        login.status.success(),
        "login failed: {}",
        String::from_utf8_lossy(&login.stderr)
    );

    let out = home.run(&["watch", "--once", "--json"]);
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
    assert!(
        memories.iter().any(|name| name.starts_with(&id)),
        "the announced memory must be on disk after the triggered pull: {memories:?}"
    );
}

#[test]
fn watch_without_a_login_is_a_usage_error() {
    let home = Home::new();
    let out = home.run(&["watch", "--once"]);
    assert!(!out.status.success(), "watch must refuse without auth.json");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("auth login"),
        "the error must point at login: {stderr}"
    );
}
