#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror tests for `src/output/prune.rs`. The full TTY/JSON shape of
//! `comemory prune --json` is covered end-to-end in `tests/cli__prune.rs`;
//! this module exists to satisfy the tests-mirror gate and to lock in
//! that `output::prune::emit` accepts an empty report and a populated
//! `Page`-wrapped report in both render modes without panicking.

use comemory::output::page::Page;
use comemory::output::prune;
use comemory::output::prune::{PruneRow, Report};

/// Build one [`PruneRow`] fixture with a fixed activation/age, so tests only
/// need to vary id and reason.
fn row(id: &str, reason: &str) -> PruneRow {
    PruneRow {
        id: id.to_string(),
        title: format!("title for {id}"),
        reason: reason.to_string(),
        activation: 0.0,
        age_days: 30,
    }
}

#[test]
fn emit_accepts_empty_report_in_json_mode() {
    let report = Report {
        orphan_edges: 0,
        stale_code_files: Page::from_slice(Vec::new(), 50, 0),
        low_value_memories: Page::from_slice(Vec::new(), 50, 0),
        ghost_ref_memories: Page::from_slice(Vec::new(), 50, 0),
        trash_count: 0,
        reclaimable_bytes: 0,
        derived_stale: false,
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
        trash_count: 0,
        reclaimable_bytes: 0,
        derived_stale: false,
    };
    prune::emit(&report, false).expect("emit must succeed for empty report (TTY)");
}

#[test]
fn emit_accepts_populated_low_value_list_in_tty_mode() {
    let report = Report {
        orphan_edges: 1,
        stale_code_files: Page::from_slice(vec!["demo:src/old.rs".into()], 50, 0),
        low_value_memories: Page::from_slice(
            vec![row("aaaa0001", "low value"), row("aaaa0002", "orphan")],
            50,
            0,
        ),
        ghost_ref_memories: Page::from_slice(vec![row("aaaa0003", "stale code")], 50, 0),
        trash_count: 2,
        reclaimable_bytes: 4096,
        derived_stale: false,
    };
    prune::emit(&report, false).expect("emit must succeed for populated report (TTY)");
}

#[test]
fn populated_ghost_ref_memories_appear_in_rendered_json() {
    // `emit(report, true)` serialises the Report straight to stdout via the
    // same `serde_json` call; assert the populated ghost-ref rows actually
    // land in that JSON (the shape `--json` consumers read), not merely that
    // emit does not panic.
    let report = Report {
        orphan_edges: 0,
        stale_code_files: Page::from_slice(Vec::new(), 50, 0),
        low_value_memories: Page::from_slice(Vec::new(), 50, 0),
        ghost_ref_memories: Page::from_slice(
            vec![row("ghost001", "stale code"), row("ghost002", "stale code")],
            50,
            0,
        ),
        trash_count: 0,
        reclaimable_bytes: 0,
        derived_stale: false,
    };
    let rendered = serde_json::to_value(&report).expect("serialise report");
    let items = rendered["ghost_ref_memories"]["items"]
        .as_array()
        .expect("ghost_ref_memories.items must be an array");
    let ids: Vec<&str> = items.iter().filter_map(|v| v["id"].as_str()).collect();
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
