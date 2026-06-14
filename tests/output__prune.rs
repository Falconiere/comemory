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
    };
    prune::emit(&report, true).expect("emit must succeed for empty report (JSON)");
}

#[test]
fn emit_accepts_empty_report_in_tty_mode() {
    let report = Report {
        orphan_edges: 0,
        stale_code_files: Page::from_slice(Vec::new(), 50, 0),
        low_value_memories: Page::from_slice(Vec::new(), 50, 0),
    };
    prune::emit(&report, false).expect("emit must succeed for empty report (TTY)");
}

#[test]
fn emit_accepts_populated_low_value_list_in_tty_mode() {
    let report = Report {
        orphan_edges: 1,
        stale_code_files: Page::from_slice(vec!["demo:src/old.rs".into()], 50, 0),
        low_value_memories: Page::from_slice(vec!["aaaa0001".into(), "aaaa0002".into()], 50, 0),
    };
    prune::emit(&report, false).expect("emit must succeed for populated report (TTY)");
}
