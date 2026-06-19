//! Mirror tests for `src/output/prune.rs`. The full TTY/JSON shape of
//! `comemory prune --json` is covered end-to-end in `tests/cli__prune.rs`;
//! this module exists to satisfy the tests-mirror gate and to lock in
//! that `output::prune::emit` accepts an empty report and a populated
//! `Page`-wrapped report in both render modes without panicking.

use comemory::cli::prune::Report;
use comemory::output::page::Page;
use comemory::output::prune;

#[test]
fn emit_accepts_empty_report_in_json_mode() {
    let report = Report {
        orphan_edges: 0,
        stale_code_files: Page::from_slice(Vec::new(), 50, 0),
        low_value_memories: Page::from_slice(Vec::new(), 50, 0),
        ghost_ref_memories: Page::from_slice(Vec::new(), 50, 0),
    };
    prune::emit(&report, true).expect("emit must succeed for empty report (JSON)");
}

#[test]
fn emit_accepts_empty_report_in_tty_mode() {
    let report = Report {
        orphan_edges: 0,
        stale_code_files: Page::from_slice(Vec::new(), 50, 0),
        low_value_memories: Page::from_slice(Vec::new(), 50, 0),
        ghost_ref_memories: Page::from_slice(Vec::new(), 50, 0),
    };
    prune::emit(&report, false).expect("emit must succeed for empty report (TTY)");
}

#[test]
fn emit_accepts_populated_low_value_list_in_tty_mode() {
    let report = Report {
        orphan_edges: 1,
        stale_code_files: Page::from_slice(vec!["demo:src/old.rs".into()], 50, 0),
        low_value_memories: Page::from_slice(vec!["aaaa0001".into(), "aaaa0002".into()], 50, 0),
        ghost_ref_memories: Page::from_slice(vec!["aaaa0003".into()], 50, 0),
    };
    prune::emit(&report, false).expect("emit must succeed for populated report (TTY)");
}

#[test]
fn populated_ghost_ref_memories_appear_in_rendered_json() {
    // `emit(report, true)` serialises the Report straight to stdout via the
    // same `serde_json` call; assert the populated ghost-ref ids actually land
    // in that JSON (the shape `--json` consumers read), not merely that emit
    // does not panic.
    let report = Report {
        orphan_edges: 0,
        stale_code_files: Page::from_slice(Vec::new(), 50, 0),
        low_value_memories: Page::from_slice(Vec::new(), 50, 0),
        ghost_ref_memories: Page::from_slice(vec!["ghost001".into(), "ghost002".into()], 50, 0),
    };
    let rendered = serde_json::to_value(&report).expect("serialise report");
    let items = rendered["ghost_ref_memories"]["items"]
        .as_array()
        .expect("ghost_ref_memories.items must be an array");
    let ids: Vec<&str> = items.iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(
        ids,
        vec!["ghost001", "ghost002"],
        "populated ghost_ref ids must appear in the rendered JSON, got {rendered}"
    );
    assert_eq!(
        rendered["ghost_ref_memories"]["total"].as_u64(),
        Some(2),
        "ghost_ref_memories page total must reflect the populated list"
    );
}
