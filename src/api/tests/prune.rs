#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/prune.rs`. Seeds prune-eligible memories via the
//! real binary plus the shared `cli_prune_support` doctoring helper, then
//! calls `api::prune::run` directly against a `Ctx` opened on the same
//! data-dir — proving the scan report and that `apply` acts on the FULL
//! low-value candidate set even when the display window is narrower
//! (`cli::prune::run` is byte-compat tested against CLI stdout in
//! `tests/cli__prune.rs`; the HTTP route — `GET`, `apply` always forced
//! `false` — lives in `tests/serve__routes__maint__prune.rs`).

use crate::test_common::cli_prune_support as support;

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::store::connection;
use support::{make_prune_eligible, save_memory};
use tempfile::TempDir;

fn data_dir(home: &TempDir) -> std::path::PathBuf {
    home.path().join(".comemory")
}

fn request() -> api::prune::Request {
    api::prune::Request {
        apply: false,
        limit: 50,
        offset: 0,
    }
}

#[test]
fn run_reports_low_value_memory_without_deleting() {
    let home = TempDir::new().expect("tempdir");
    let id = save_memory(&home, "stale prune candidate body");
    make_prune_eligible(&home, &id);

    let paths = Paths::new(data_dir(&home));
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let report = api::prune::run(&mut ctx, request()).expect("prune run");
    assert_eq!(report.low_value_memories.items.len(), 1);
    let row = &report.low_value_memories.items[0];
    assert_eq!(row.id, id);
    assert_eq!(row.reason, "low value");
    assert_eq!(report.low_value_memories.total, Some(1));
    assert_eq!(report.trash_count, 0);
    assert_eq!(report.reclaimable_bytes, 0);

    let trash = data_dir(&home).join("memories").join(".trash");
    let trashed = std::fs::read_dir(&trash)
        .map(std::iter::Iterator::count)
        .unwrap_or_default();
    assert_eq!(trashed, 0, "dry-run must not move files to .trash");
}

#[test]
fn run_apply_soft_deletes_the_full_low_value_set_even_with_a_narrow_window() {
    let home = TempDir::new().expect("tempdir");
    let a = save_memory(&home, "doomed prune candidate one");
    let b = save_memory(&home, "doomed prune candidate two");
    make_prune_eligible(&home, &a);
    make_prune_eligible(&home, &b);

    let paths = Paths::new(data_dir(&home));
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    // limit=1 windows the DISPLAY only — apply must still act on both ids.
    let req = api::prune::Request {
        apply: true,
        limit: 1,
        offset: 0,
    };
    let report = api::prune::run(&mut ctx, req).expect("prune run");
    assert_eq!(
        report.low_value_memories.items.len(),
        1,
        "display window stays narrow"
    );
    assert_eq!(report.low_value_memories.total, Some(2));

    let memories = data_dir(&home).join("memories");
    for flagged in [&a, &b] {
        let live = std::fs::read_dir(&memories)
            .expect("read memories dir")
            .filter_map(std::result::Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(flagged.as_str())
            })
            .count();
        assert_eq!(live, 0, "{flagged} must leave memories/");
    }
    let trashed = std::fs::read_dir(memories.join(".trash"))
        .expect("read .trash")
        .count();
    assert_eq!(trashed, 2, "both flagged ids must land in .trash/");
}
