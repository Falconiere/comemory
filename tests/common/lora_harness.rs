#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    // One harness, two consuming binaries: the recipe suite never scores an
    // arm and the qualification suite never tampers with a split, so each
    // compiles items the other uses. Same convention as
    // `tests/common/export_dataset_corpus.rs`.
    dead_code
)]
//! Shared harness for `tests/lora_qualification.rs` and
//! `tests/lora_qualification_2.rs`: the mixed corpus
//! `tests/common/export_dataset_corpus.rs` builds, extended with a third
//! lexically isolated memory and its own reviewed verdict, plus the `python3`
//! plumbing both suites drive the recipe and the harness through.
//!
//! Three connected components is the minimum that lets one export fill train,
//! validation and holdout at once. With two, every split-boundary assertion
//! would be vacuous, because one component is one group and one group is one
//! split.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[path = "export_dataset_corpus.rs"]
pub mod fixture;

pub use fixture::corpus::Home;
use fixture::corpus::ref_in;
pub use fixture::out_of;
use fixture::{export, prepared};
use serde_json::Value;

/// A memory whose vocabulary overlaps nothing else in the fixture corpus, so
/// the query below forms a third connected component. Three components is the
/// minimum that lets one export fill train, validation and holdout at once,
/// which is what the split-boundary assertions need.
const THIRD: &str = "Gribbleflax tessellation index.\n\nThe gribbleflax tessellation index shares \
                     no vocabulary with anything else this corpus holds.";

/// The query that reaches only [`THIRD`].
const THIRD_QUERY: &str = "gribbleflax tessellation";

/// The seeds the split search walks. The grouped-hash splitter assigns a whole
/// component by `sha256(seed NUL group_id)`, so which seed separates three
/// components is arbitrary but deterministic; the search makes the fixture
/// robust without making it random.
const SEEDS: [&str; 24] = [
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s",
    "t", "u", "v", "w", "x",
];

/// The repository root, resolved the way every data-file consumer here does.
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// One `python3` invocation from the repository root.
pub fn python(args: &[&str]) -> Output {
    let out = Command::new("python3")
        .args(args)
        .current_dir(repo_root())
        .output();
    match out {
        Ok(out) => out,
        Err(e) => panic!(
            "python3 is a prerequisite of this suite and could not be started ({e}). \
             It is not skipped: a skipped test that reads as a pass is worse than no test."
        ),
    }
}

/// One `python3` invocation that must succeed.
pub fn python_ok(args: &[&str]) -> String {
    let out = python(args);
    assert!(
        out.status.success(),
        "python3 {args:?} failed ({:?}): {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("stdout utf8")
}

/// The training entry point's path, as an argument.
pub fn train_py() -> String {
    "integrations/training/comemory_train.py".to_string()
}

/// The qualification entry point's path, as an argument.
pub fn qualify_py() -> String {
    "integrations/training/comemory_qualify.py".to_string()
}

/// The reference backend's entry point, which the scorer arms drive.
pub fn backend_py() -> String {
    "integrations/reranker/comemory_rerank.py".to_string()
}

/// A corpus with three judged observations in three connected components.
///
/// `prepared` already captures two runs and judges the first. The second — the
/// lexically isolated query — is captured but unjudged, and an unjudged
/// observation contributes no exported row and therefore no group, so judging
/// it is what turns two components into three. A third isolated memory and its
/// own query supply the last one. Re-running either query instead of judging
/// the existing observation would mint a DUPLICATE observation, which the
/// exporter drops, taking the verdict with it.
pub fn three_judged() -> Home {
    let p = prepared();
    p.home
        .run_ok(&["save", THIRD, "--kind", "note", "--repo", "demo"]);
    let third = observation_for(&p.home, THIRD_QUERY);
    let isolated = observation_by_query(&p.home, fixture::ISOLATED_QUERY);
    for observation in [isolated, third] {
        let conn = p.home.db();
        let candidate = ref_in(&conn, &observation, "memory");
        drop(conn);
        p.home
            .run_ok(&["judge", &observation, "--ref", &format!("{candidate}=3")]);
    }
    p.home
}

/// Capture one run of `query` and return the observation it recorded.
pub fn observation_for(home: &Home, query: &str) -> String {
    let found = home.capture_json(&["find", query, "--k", "20"]);
    found["observation_id"]
        .as_str()
        .unwrap_or_else(|| panic!("find {query} reported no observation_id: {found}"))
        .to_string()
}

/// The observation an earlier captured run of `query` already recorded.
pub fn observation_by_query(home: &Home, query: &str) -> String {
    let conn = home.db();
    conn.query_row(
        "SELECT observation_id FROM candidate_query_observations WHERE query = ?1 \
          ORDER BY observation_id LIMIT 1",
        [query],
        |row| row.get::<_, String>(0),
    )
    .unwrap_or_else(|e| panic!("no captured observation for {query}: {e}"))
}

/// Export into `out` with a seed that fills every split named in `wanted`.
pub fn export_filling(home: &Home, out: &Path, ratios: &str, wanted: &[&str]) -> Value {
    for seed in SEEDS {
        let manifest = export(
            home,
            out,
            &["--split", ratios, "--split-seed", seed, "--include-holdout"],
        );
        if wanted.iter().all(|split| {
            manifest["by_split"][*split]
                .as_i64()
                .is_some_and(|rows| rows > 0)
        }) {
            return manifest;
        }
    }
    panic!(
        "no seed in {} filled {}; the fixture has too few components",
        SEEDS.join(", "),
        wanted.join(", ")
    );
}

/// Read one JSON file.
pub fn read_json(path: &Path) -> Value {
    let body =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()))
}

/// Generate a benchmark set from `dataset`'s holdout and return
/// `(set, provenance)`. Both suites need it and neither owns it.
pub fn build_set(workspace: &Path, dataset: &Path, min_tasks: &str) -> (PathBuf, PathBuf) {
    let set = workspace.join("set.yaml");
    let provenance = workspace.join("set.provenance.json");
    python_ok(&[
        &qualify_py(),
        "build-set",
        "--dataset",
        dataset.to_str().expect("utf8"),
        "--out",
        set.to_str().expect("utf8"),
        "--provenance",
        provenance.to_str().expect("utf8"),
        "--min-tasks",
        min_tasks,
    ]);
    (set, provenance)
}

/// Run `comemory benchmark` over `set` and return the artifact's path.
pub fn benchmark(home: &Home, set: &Path, report: &Path, scores: &[&Path]) -> String {
    let mut args: Vec<String> = vec![
        "benchmark".into(),
        "--set".into(),
        set.to_str().expect("utf8").into(),
        "--report".into(),
        report.to_str().expect("utf8").into(),
    ];
    for path in scores {
        args.push("--scores".into());
        args.push(path.to_str().expect("utf8").into());
    }
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    home.run_ok(&borrowed)
}
