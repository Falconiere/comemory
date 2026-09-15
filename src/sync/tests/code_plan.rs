#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The pure diff and batching behind the code push.

use comemory::api::sync::{CodeFileRef, CodeFileWire, CodeSymbolWire};
use comemory::sync::code_plan::{MAX_BATCH_FILES, batches, plan};

fn local(rows: &[(&str, &str)]) -> Vec<(String, String)> {
    rows.iter()
        .map(|(p, o)| ((*p).to_owned(), (*o).to_owned()))
        .collect()
}

fn remote(rows: &[(&str, &str)]) -> Vec<CodeFileRef> {
    rows.iter()
        .map(|(p, o)| CodeFileRef {
            path: (*p).to_owned(),
            blob_oid: (*o).to_owned(),
        })
        .collect()
}

#[test]
fn identical_sides_plan_nothing() {
    let p = plan(
        &local(&[("a", "1"), ("b", "2")]),
        &remote(&[("a", "1"), ("b", "2")]),
        Some("h"),
        Some("h"),
        Some("m"),
        Some("m"),
    );
    assert!(p.is_empty(), "{p:?}");
}

#[test]
fn changed_absent_and_removed_are_told_apart() {
    let p = plan(
        &local(&[("a", "1"), ("b", "2x"), ("d", "4")]),
        &remote(&[("a", "1"), ("b", "2"), ("c", "3")]),
        Some("h2"),
        Some("h1"),
        Some("m"),
        None,
    );
    assert_eq!(p.changed, ["b", "d"]);
    assert_eq!(p.removed, ["c"]);
    assert!(p.send_cochange, "remote has no mining cursor yet");
    assert!(p.head_moved);
}

#[test]
fn a_never_mined_repo_sends_no_cochange() {
    let p = plan(&local(&[]), &remote(&[]), None, None, None, Some("stale"));
    assert!(!p.send_cochange);
    assert!(p.is_empty());
}

fn file(path: &str, symbols: usize) -> CodeFileWire {
    CodeFileWire {
        path: path.into(),
        blob_oid: "0".repeat(40),
        symbols: (0..symbols)
            .map(|i| CodeSymbolWire {
                symbol: format!("s{i}"),
                kind: "function".into(),
                lang: "typescript".into(),
                line_start: 1,
                line_end: 2,
            })
            .collect(),
        imports: Vec::new(),
    }
}

#[test]
fn batches_split_by_file_count_and_stamp_the_marker_on_the_last_only() {
    let files: Vec<CodeFileWire> = (0..=MAX_BATCH_FILES)
        .map(|i| file(&format!("f{i}"), 1))
        .collect();
    let out = batches(
        "r",
        Some("h"),
        Some("m"),
        files,
        vec!["gone".into()],
        Some(Vec::new()),
    )
    .unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].files.len(), MAX_BATCH_FILES);
    assert_eq!(out[1].files.len(), 1);
    assert_eq!(out[0].removed, ["gone"]);
    assert!(out[1].removed.is_empty());
    // A push that dies after batch 0 must leave the workspace's marker and
    // co-change set untouched: only the last batch carries them.
    assert_eq!(out[0].head, None);
    assert_eq!(out[0].mined_commit, None);
    assert!(out[0].cochange.is_none());
    assert_eq!(out[1].head.as_deref(), Some("h"));
    assert_eq!(out[1].mined_commit.as_deref(), Some("m"));
    assert!(out[1].cochange.is_some());
}

#[test]
fn batches_split_by_bytes() {
    // ~200 symbols × ~90 bytes each ≈ 18 KB per file; 80 files ≈ 1.4 MB.
    let files: Vec<CodeFileWire> = (0..80).map(|i| file(&format!("f{i}"), 200)).collect();
    let out = batches("r", None, None, files, Vec::new(), None).unwrap();
    assert!(out.len() >= 2, "{} batches", out.len());
    let total: usize = out.iter().map(|b| b.files.len()).sum();
    assert_eq!(total, 80);
}

#[test]
fn an_empty_plan_still_yields_one_request() {
    let out = batches("r", Some("h"), None, Vec::new(), Vec::new(), None).unwrap();
    assert_eq!(out.len(), 1);
    assert!(out[0].files.is_empty());
}

#[test]
fn removals_that_outnumber_file_chunks_still_fill_every_batch() {
    // One file chunk, three removal chunks: every batch carries something,
    // and the marker still rides the last one only.
    let removed: Vec<String> = (0..=(MAX_BATCH_FILES * 2))
        .map(|i| format!("gone{i}"))
        .collect();
    let out = batches(
        "r",
        Some("h"),
        Some("m"),
        vec![file("kept", 1)],
        removed,
        None,
    )
    .unwrap();
    assert_eq!(out.len(), 3);
    assert!(
        out.iter()
            .all(|b| !b.files.is_empty() || !b.removed.is_empty()),
        "an empty batch would be a wasted round trip"
    );
    assert_eq!(out[0].files.len(), 1);
    assert_eq!(out[0].removed.len(), MAX_BATCH_FILES);
    assert_eq!(out[2].removed.len(), 1);
    assert!(
        out[..2]
            .iter()
            .all(|b| b.head.is_none() && b.mined_commit.is_none())
    );
    assert_eq!(out[2].head.as_deref(), Some("h"));
}
