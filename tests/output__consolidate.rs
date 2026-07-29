//! Mirror for `comemory::output::consolidate` — the TTY cluster blocks, the
//! member cap, and the JSON passthrough, exercised through the real binary
//! so the assertions read exactly what an operator sees.

use assert_cmd::Command;
use tempfile::TempDir;

/// Build a `comemory` invocation with `COMEMORY_DATA_DIR` rooted at `home`.
fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

/// Save one memory through the binary.
fn save(home: &TempDir, body: &str) {
    bin(home)
        .args(["--json", "save", body, "--kind", "note", "--repo", "demo"])
        .assert()
        .success();
}

/// `consolidate` stdout in TTY mode.
fn tty(home: &TempDir, args: &[&str]) -> String {
    let assertion = bin(home).arg("consolidate").args(args).assert().success();
    String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout")
}

/// `n` memories that all differ only in a trailing ordinal word, so a wide
/// radius unions every one of them into a single cluster.
fn wide_corpus(n: usize) -> TempDir {
    let home = TempDir::new().expect("tempdir");
    for i in 0..n {
        save(
            &home,
            &format!("postgres advisory lock ordering fix number {i} for the migration runner"),
        );
    }
    home
}

#[test]
fn a_cluster_block_names_its_size_distance_and_keeper() {
    let home = TempDir::new().expect("tempdir");
    save(
        &home,
        "redis cache eviction policy tuning for the session store",
    );
    save(
        &home,
        "redis cache eviction policy tuning for the session cache",
    );

    let out = tty(&home, &[]);
    assert!(
        out.contains("clusters : 1  (2 memories scanned, radius 8)"),
        "header must carry the count, scan size and radius: {out}"
    );
    assert!(
        out.contains("cluster 1  (2 members, max hamming"),
        "cluster header missing its size and widest distance: {out}"
    );
    assert!(out.contains("← keeper"), "the keeper is unmarked: {out}");
    assert!(
        out.contains("quality 3  accessed 0"),
        "member lines carry the stats behind the order: {out}"
    );
    assert!(
        out.contains("hamming "),
        "non-keepers report their distance: {out}"
    );
}

#[test]
fn the_member_list_is_capped_and_the_tail_counts_the_remainder() {
    let home = wide_corpus(25);
    let out = tty(&home, &["--radius", "40"]);
    assert!(
        out.contains("cluster 1  (25 members"),
        "the cluster itself is not truncated: {out}"
    );
    assert!(
        out.contains("… and 5 more"),
        "20 rendered of 25 leaves a tail of 5: {out}"
    );
    let rendered = out.matches("  quality ").count();
    assert_eq!(rendered, 20, "exactly the cap is rendered: {out}");
}

#[test]
fn json_carries_every_member_the_tty_capped() {
    let home = wide_corpus(25);
    let assertion = bin(&home)
        .args(["--json", "consolidate", "--radius", "40"])
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    let members = v["clusters"]["items"][0]["members"]
        .as_array()
        .expect("members array");
    assert_eq!(
        members.len(),
        25,
        "the machine-readable report is never truncated: {v}"
    );
}

#[test]
fn a_resolved_cluster_is_labelled_and_drops_the_retired_members_from_the_hint() {
    let home = TempDir::new().expect("tempdir");
    save(
        &home,
        "redis cache eviction policy tuning for the session store",
    );
    let assertion = bin(&home).args(["--json", "list"]).assert().success();
    let listed = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(listed.trim()).expect("parse list JSON");
    let first = v["items"][0]["id"].as_str().expect("id").to_string();

    bin(&home)
        .args([
            "--json",
            "save",
            "redis cache eviction policy tuning for the session cache",
            "--kind",
            "note",
            "--repo",
            "demo",
            "--supersedes",
            &first,
        ])
        .assert()
        .success();

    let out = tty(&home, &["--all"]);
    assert!(
        out.contains(", resolved)"),
        "an already-handled cluster is labelled: {out}"
    );
    assert!(
        !out.contains("merge with"),
        "nothing left to merge, so no hint: {out}"
    );
}

#[test]
fn an_unfingerprinted_corpus_is_called_out_in_the_header() {
    let home = TempDir::new().expect("tempdir");
    save(&home, "graphql federation gateway timeout budget");
    let db = home.path().join(".comemory").join("comemory.db");
    let conn = comemory::store::connection::open(&db).expect("open mirror");
    conn.execute("UPDATE memories SET simhash = 0", [])
        .expect("clear fingerprints");
    drop(conn);

    let out = tty(&home, &[]);
    assert!(
        out.contains("1 memories have no fingerprint yet"),
        "a half-migrated store must be visible, not silently empty: {out}"
    );
    assert!(out.contains("comemory rebuild"), "and actionable: {out}");
}

#[test]
fn the_empty_report_still_prints_a_footer() {
    let home = TempDir::new().expect("tempdir");
    let out = tty(&home, &[]);
    assert!(out.contains("clusters : 0"), "zero is stated: {out}");
    assert!(
        !out.contains("cluster 1"),
        "no cluster blocks are invented: {out}"
    );
}
