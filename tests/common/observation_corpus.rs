#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    // One fixture, three consuming binaries: the capture suite never edits the
    // corpus, the judgment suite never inspects a bound, and the export suite
    // uses neither the edited variants nor the counters, so each compiles
    // items the others use. Same convention as `tests/common/auth_fixture.rs`.
    dead_code
)]
//! Shared fixture for `tests/cli__judge.rs`, `tests/cli__judge_2.rs` and
//! `tests/cli__export_dataset.rs`: a real mixed-domain corpus driven through
//! the real `comemory` binary, plus the database reads those suites assert
//! against.
//!
//! Three memories saved through the production save path, one real git
//! repository indexed with `index-code`, and one real markdown tree indexed
//! with `index` — the same shape `tests/cli__benchmark.rs` builds, because the
//! whole point of a candidate observation is that all three domains reach one
//! pool.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use rusqlite::Connection;
use serde_json::Value;
use tempfile::TempDir;

/// The git helpers this fixture seeds with, included here rather than by each
/// consuming binary so `seeded` needs no injected callbacks and the two suites
/// share one definition of the corpus.
#[path = "git_commit.rs"]
pub mod git_commit;
#[path = "git_repo.rs"]
pub mod git_repo;

/// The query every corpus in this fixture answers: "activation" appears in two
/// memory bodies, in the indexed symbol's own name, and in the indexed
/// document. A query only one corpus answers would make every cross-domain
/// assertion in either suite vacuous.
pub const QUERY: &str = "activation";

/// Env var that arms candidate capture for one subprocess.
pub const CAPTURE_ON: (&str, &str) = ("COMEMORY_OBSERVATIONS_ENABLED", "1");

/// The three fixture memory bodies. Two of them mention "activation", which
/// is also the indexed symbol's name stem and a word in the indexed document,
/// so one `find activation` reaches all three corpora and pools more than one
/// memory — without that, every page-versus-pool assertion would be vacuous.
pub const MEMORIES: [(&str, &str); 3] = [
    (
        "bug",
        "Paged search reorders tied results.\n\nAccess tracking bumps only the returned page, so a gapped bump set floats over the rows it was behind while activation decays.",
    ),
    (
        "decision",
        "Activation decay must be pinned for measurement.\n\nDisabling access tracking does not freeze activation decay; only a zero decay exponent removes the wall clock.",
    ),
    (
        "note",
        "Document chunking splits on paragraph boundaries.\n\nThe shared splitter keeps a heading breadcrumb for every passage it emits.",
    ),
];

/// A rust source file carrying the symbol the code-domain assertions judge.
pub const RANKING_RS: &str = "\
/// Map activation to a bounded multiplier with a clamp.
pub fn activation_boost(activation: f64, clamp: (f64, f64)) -> f64 {
    let raw = (0.2 * activation).exp();
    raw.max(clamp.0).min(clamp.1)
}
";

/// The same file after an edit: a new blob OID for the same symbol.
pub const RANKING_RS_EDITED: &str = "\
/// Map activation to a bounded multiplier with a clamp.
pub fn activation_boost(activation: f64, clamp: (f64, f64)) -> f64 {
    let raw = (0.25 * activation).exp();
    raw.max(clamp.0).min(clamp.1)
}
";

/// The markdown document the document-domain assertions judge.
pub const CHUNKING_MD: &str = "\
# Chunking and activation

The shared splitter breaks a document on a paragraph boundary and keeps the
heading breadcrumb for every passage it emits, so a chunk cites where it came
from even when activation decay reorders the page.
";

/// The same document rewritten: a new `revision_hash` for the same path.
pub const CHUNKING_MD_EDITED: &str = "\
# Chunking and activation

The splitter now breaks on a heading boundary first and only then on a
paragraph, so activation decay sees a different passage for the same path.
";

/// A throwaway data directory plus the workspace the fixtures live in.
pub struct Home {
    root: TempDir,
}

impl Home {
    /// Fresh tempdir. `<root>/.comemory` is the data dir.
    pub fn new() -> Self {
        Self {
            root: TempDir::new().expect("tempdir"),
        }
    }

    /// The `COMEMORY_DATA_DIR` this home points every command at.
    pub fn data_dir(&self) -> PathBuf {
        self.root.path().join(".comemory")
    }

    /// The workspace root holding the git repo and the markdown tree.
    pub fn workspace(&self) -> &Path {
        self.root.path()
    }

    /// `comemory` with the data dir set and capture OFF.
    pub fn bin(&self) -> Command {
        let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
        c.env("COMEMORY_DATA_DIR", self.data_dir());
        c
    }

    /// `comemory` with the data dir set and candidate capture armed.
    pub fn capturing_bin(&self) -> Command {
        let mut c = self.bin();
        c.env(CAPTURE_ON.0, CAPTURE_ON.1);
        c
    }

    /// Run `comemory <args>` to success and return stdout.
    pub fn run_ok(&self, args: &[&str]) -> String {
        let out = self.bin().args(args).output().expect("run comemory");
        assert!(
            out.status.success(),
            "comemory {args:?} failed ({:?}): {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("stdout utf8")
    }

    /// Run `comemory --json <args>` to success and parse stdout.
    pub fn run_json(&self, args: &[&str]) -> Value {
        parse_json(&self.run_ok(&with_json(args)))
    }

    /// Run `comemory --json <args>` with capture armed and parse stdout.
    pub fn capture_json(&self, args: &[&str]) -> Value {
        let out = self
            .capturing_bin()
            .args(with_json(args))
            .output()
            .expect("run comemory");
        assert!(
            out.status.success(),
            "comemory {args:?} failed ({:?}): {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        parse_json(&String::from_utf8_lossy(&out.stdout))
    }

    /// Run `comemory <args>` expecting failure; returns `(exit code, stderr)`.
    pub fn run_err(&self, args: &[&str]) -> (Option<i32>, String) {
        let out = self.bin().args(args).output().expect("run comemory");
        assert!(
            !out.status.success(),
            "comemory {args:?} unexpectedly succeeded: {}",
            String::from_utf8_lossy(&out.stdout)
        );
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// A read-write connection to the live `comemory.db`.
    pub fn db(&self) -> Connection {
        Connection::open(self.data_dir().join("comemory.db")).expect("open comemory.db")
    }
}

fn with_json<'a>(args: &'a [&'a str]) -> Vec<&'a str> {
    let mut v = Vec::with_capacity(args.len() + 1);
    v.push("--json");
    v.extend_from_slice(args);
    v
}

fn parse_json(stdout: &str) -> Value {
    serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout is not JSON: {e}\n{stdout}"))
}

/// Seed the mixed corpus: three memories, one indexed git repo, one indexed
/// markdown tree. Returns the home and the repository path.
pub fn seeded() -> (Home, PathBuf) {
    let home = Home::new();
    for (kind, body) in MEMORIES {
        home.run_ok(&["save", body, "--kind", kind, "--repo", "demo"]);
    }
    let repo = home.workspace().join("demo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[("src/ranking.rs", RANKING_RS)], "ranking");
    home.run_ok(&[
        "index-code",
        "--path",
        repo.to_str().expect("utf8"),
        "--repo",
        "demo",
    ]);
    let docs = home.workspace().join("guides");
    std::fs::create_dir_all(&docs).expect("create docs dir");
    std::fs::write(docs.join("chunking.md"), CHUNKING_MD).expect("write doc");
    home.run_ok(&["index", docs.to_str().expect("utf8")]);
    (home, repo)
}

/// Every candidate reference of `observation_id`, ascending by pool position.
pub fn refs(conn: &Connection, observation_id: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT candidate_ref FROM candidate_observations \
              WHERE observation_id = ?1 ORDER BY pool_position",
        )
        .expect("prepare");
    stmt.query_map([observation_id], |r| r.get::<_, String>(0))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("rows")
}

/// The first candidate reference of `observation_id` whose domain is `domain`.
pub fn ref_in(conn: &Connection, observation_id: &str, domain: &str) -> String {
    let mut stmt = conn
        .prepare(
            "SELECT candidate_ref FROM candidate_observations \
              WHERE observation_id = ?1 AND domain = ?2 ORDER BY pool_position LIMIT 1",
        )
        .expect("prepare");
    stmt.query_row(rusqlite::params![observation_id, domain], |r| r.get(0))
        .unwrap_or_else(|e| panic!("no {domain} candidate in {observation_id}: {e}"))
}

/// One scalar of one candidate row.
pub fn candidate_scalar<T: rusqlite::types::FromSql>(
    conn: &Connection,
    observation_id: &str,
    candidate_ref: &str,
    column: &str,
) -> T {
    let sql = format!(
        "SELECT {column} FROM candidate_observations \
          WHERE observation_id = ?1 AND candidate_ref = ?2"
    );
    conn.query_row(
        &sql,
        rusqlite::params![observation_id, candidate_ref],
        |r| r.get(0),
    )
    .unwrap_or_else(|e| panic!("read {column} for {candidate_ref}: {e}"))
}

/// Rows in `table`.
pub fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap_or_else(|e| panic!("count {table}: {e}"))
}

/// The `observation_id` of a `find --json` response, which must be present.
pub fn observation_of(found: &Value) -> String {
    found["observation_id"]
        .as_str()
        .unwrap_or_else(|| panic!("find reported no observation_id: {found}"))
        .to_string()
}
