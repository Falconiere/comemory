//! Shared corpus for the time-travel CLI tests (`--since` / `--until` /
//! `--as-of` on `comemory search` and `comemory context`).
//!
//! Two memories that both match the query "advisory locks", wired into a
//! supersede chain, with hand-set `created` dates:
//!
//! | memory | created | relation |
//! |---|---|---|
//! | [`OLDER_BODY`] | [`OLDER_CREATED`] | superseded by the newer one |
//! | [`NEWER_BODY`] | [`NEWER_CREATED`] | supersedes the older one |
//!
//! The dates are stamped by rewriting the markdown frontmatter after
//! `comemory save` and then running the real `comemory rebuild`, which
//! re-materializes the supersede edge with a *fresh* `edges.created_at`.
//! That is deliberate: it is what makes the as-of tests prove the penalty
//! scoping keys on the superseder memory's `created_at` (frontmatter,
//! survives rebuild) rather than the edge's (rebuild-stamped "now").

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;
use tempfile::{TempDir, tempdir};

/// Body of the older memory, superseded by [`NEWER_BODY`].
pub const OLDER_BODY: &str = "postgres advisory locks for migration ordering";
/// Body of the newer memory, which supersedes [`OLDER_BODY`].
pub const NEWER_BODY: &str = "redis streams replace postgres advisory locks";
/// Frontmatter `created` date stamped onto the older memory.
pub const OLDER_CREATED: &str = "2026-03-10";
/// Frontmatter `created` date stamped onto the newer memory.
pub const NEWER_CREATED: &str = "2026-06-01";
/// Query both memories match lexically.
pub const QUERY: &str = "advisory locks";

/// The seeded corpus: a data dir plus the ids of both memories.
pub struct Corpus {
    /// Temp `COMEMORY_DATA_DIR` holding `memories/` + `comemory.db`.
    pub home: TempDir,
    /// Id of the older, superseded memory ([`OLDER_BODY`]).
    pub older: String,
    /// Id of the newer, superseding memory ([`NEWER_BODY`]).
    pub newer: String,
}

impl Corpus {
    /// Run `comemory <sub> "<QUERY>" --json [args…]` over this corpus and
    /// parse the envelope. Access tracking is disabled so repeated runs
    /// cannot reorder each other through the activation prior.
    pub fn run(&self, sub: &str, args: &[&str]) -> Value {
        let out = Command::cargo_bin("comemory")
            .expect("cargo_bin comemory")
            .env("COMEMORY_DATA_DIR", self.home.path())
            .env("COMEMORY_DISABLE_ACCESS_TRACKING", "true")
            .args([sub, QUERY, "--json"])
            .args(args)
            .assert()
            .success();
        parse(&out.get_output().stdout)
    }
}

/// Seed the two-memory supersede chain, stamp the crafted `created` dates,
/// and rebuild the SQLite mirror from the rewritten markdown.
pub fn seed() -> Corpus {
    let home = tempdir().expect("tempdir");
    let (older, older_path) = save(&home, OLDER_BODY, None);
    let (newer, newer_path) = save(&home, NEWER_BODY, Some(&older));
    retime(&older_path, OLDER_CREATED);
    retime(&newer_path, NEWER_CREATED);
    Command::cargo_bin("comemory")
        .expect("cargo_bin comemory")
        .env("COMEMORY_DATA_DIR", home.path())
        .arg("rebuild")
        .assert()
        .success();
    Corpus { home, older, newer }
}

/// Save one memory and return its id plus the markdown path `save --json`
/// reported.
fn save(home: &TempDir, body: &str, supersedes: Option<&str>) -> (String, PathBuf) {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path());
    c.args(["--json", "save", "--kind", "decision"]);
    if let Some(id) = supersedes {
        c.args(["--supersedes", id]);
    }
    let out = c.arg(body).assert().success();
    let v = parse(&out.get_output().stdout);
    let id = v["id"].as_str().expect("save --json emits id").to_string();
    let path = v["path"].as_str().expect("save --json emits path");
    (id, PathBuf::from(path))
}

/// Rewrite the frontmatter `created:` date of `path` to `date`
/// (`YYYY-MM-DD`), keeping the writer's time-of-day and offset so the value
/// still parses as the ISO-8601 form `comemory save` emits.
fn retime(path: &Path, date: &str) {
    let raw = std::fs::read_to_string(path).expect("read memory markdown");
    let mut out = String::with_capacity(raw.len());
    for line in raw.lines() {
        if let Some(stamped) = restamp(line, date) {
            out.push_str(&stamped);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    std::fs::write(path, out).expect("write memory markdown");
}

/// Swap the date portion of a `created:` frontmatter line, or `None` for
/// any other line. The date is the ten characters preceding the `T`.
fn restamp(line: &str, date: &str) -> Option<String> {
    let t = line.strip_prefix("created:")?.find('T')? + "created:".len();
    let mut stamped = line.to_string();
    stamped.replace_range(t - date.len()..t, date);
    Some(stamped)
}

/// Parse a `--json` stdout buffer, failing loudly with the raw text.
fn parse(stdout: &[u8]) -> Value {
    let raw = String::from_utf8_lossy(stdout).to_string();
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("expected JSON, got {raw:?}: {e}"))
}
