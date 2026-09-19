#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    // One fixture, two consuming binaries: the shape suite never inspects a
    // JSON key tree and the leakage suite never reads the isolated body, so
    // each compiles items the other uses.
    dead_code
)]
//! Shared fixture for `tests/cli__export_dataset.rs` and
//! `tests/cli__export_dataset_2.rs`: the mixed corpus
//! `tests/common/observation_corpus.rs` builds, extended with one lexically
//! isolated memory and a second capturing run so the export sees more than one
//! connected component, plus one fully judged observation.
//!
//! Without that second component every split assertion in either suite would
//! pass vacuously: one component is one group, and one group is one split.

use std::path::{Path, PathBuf};

use serde_json::Value;

#[path = "observation_corpus.rs"]
pub mod corpus;

use corpus::{Home, QUERY, observation_of, ref_in, seeded};

/// A memory whose vocabulary shares no token with the fixture corpus, so the
/// second query below forms its own connected component. Without it every row
/// would be one group and every split assertion would be vacuous.
pub const ISOLATED: &str = "Xyloph bandersnatch calibration.\n\nThe xyloph bandersnatch calibration table is unrelated \
     to anything else this corpus holds.";

/// The query that reaches only [`ISOLATED`].
pub const ISOLATED_QUERY: &str = "xyloph bandersnatch";

/// A corpus with two capturing runs and one fully judged observation.
pub struct Prepared {
    /// The seeded data directory and workspace.
    pub home: Home,
    /// The fully judged observation.
    pub first: String,
    /// The memory candidate that was graded 3.
    pub memory_ref: String,
    /// The code candidate that was graded 2.
    pub code_ref: String,
    /// The document candidate that was graded 0.
    pub document_ref: String,
}

pub fn prepared() -> Prepared {
    let (home, _repo) = seeded();
    home.run_ok(&["save", ISOLATED, "--kind", "note", "--repo", "demo"]);
    let first = observation_of(&home.capture_json(&["find", QUERY, "--k", "20"]));
    let second = observation_of(&home.capture_json(&["find", ISOLATED_QUERY, "--k", "20"]));
    assert_ne!(first, second, "two runs must capture two observations");

    let conn = home.db();
    let memory_ref = ref_in(&conn, &first, "memory");
    let code_ref = ref_in(&conn, &first, "code");
    let document_ref = ref_in(&conn, &first, "document");
    drop(conn);
    home.run_ok(&[
        "judge",
        &first,
        "--ref",
        &format!("{memory_ref}=3"),
        "--ref",
        &format!("{code_ref}=2"),
        "--ref",
        &format!("{document_ref}=0"),
    ]);
    Prepared {
        home,
        first,
        memory_ref,
        code_ref,
        document_ref,
    }
}

/// Run `comemory --json export-dataset --out <dir> <extra>` and return the
/// manifest it printed.
pub fn export(home: &Home, out: &Path, extra: &[&str]) -> Value {
    let mut args = vec!["export-dataset", "--out", out.to_str().expect("utf8")];
    args.extend_from_slice(extra);
    home.run_json(&args)
}

pub fn records(path: &Path) -> Vec<Value> {
    let body = std::fs::read_to_string(path).expect("read a jsonl file");
    body.lines()
        .map(|line| serde_json::from_str(line).expect("each line is one JSON record"))
        .collect()
}

pub fn field(records: &[Value], key: &str) -> Vec<String> {
    records
        .iter()
        .filter_map(|r| r[key].as_str().map(str::to_string))
        .collect()
}

pub fn out_of(home: &Home, name: &str) -> PathBuf {
    home.workspace().join(name)
}

/// Every object key in `value`, at every depth.
pub fn collect_keys(value: &Value, into: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                into.push(key.clone());
                collect_keys(child, into);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_keys(child, into);
            }
        }
        _ => {}
    }
}
