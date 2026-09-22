#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/maintenance/prune.rs`. Seeds prune-eligible memories via the
//! real binary plus the shared `cli_prune_support` doctoring helper, then
//! calls `maintenance::prune::run` directly against a `Ctx` opened on the same
//! data-dir — proving the scan report and that `apply` acts on the FULL
//! low-value candidate set even when the display window is narrower
//! (`cli::prune::run` is byte-compat tested against CLI stdout in
//! `tests/cli__prune.rs`; the HTTP route — `GET`, `apply` always forced
//! `false` — lives in `tests/serve__routes__maint__prune.rs`).

use crate::test_common::cli_prune_support as support;

use comemory::config::{Config, Paths};
use comemory::domains::maintenance;
use comemory::store::connection;
use comemory::utilities::context::Ctx;
use support::{make_prune_eligible, save_memory};
use tempfile::TempDir;

fn data_dir(home: &TempDir) -> std::path::PathBuf {
    home.path().join(".comemory")
}

fn request() -> maintenance::prune::Request {
    maintenance::prune::Request {
        apply: false,
        limit: 50,
        offset: 0,
        ids: Vec::new(),
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

    let report = maintenance::prune::run(&mut ctx, request()).expect("prune run");
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
    let req = maintenance::prune::Request {
        apply: true,
        limit: 1,
        offset: 0,
        ids: Vec::new(),
    };
    let report = maintenance::prune::run(&mut ctx, req).expect("prune run");
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

/// `ids` narrows `apply` to the intersection with the scan's candidates:
/// the listed candidate is soft-deleted, the unlisted one survives, and an
/// id that is not a candidate at all is ignored rather than deleted.
#[test]
fn run_apply_with_ids_touches_only_the_listed_candidate() {
    let home = TempDir::new().expect("tempdir");
    let doomed = save_memory(&home, "ids-scoped prune candidate one");
    let spared = save_memory(&home, "ids-scoped prune candidate two");
    let healthy = save_memory(&home, "a perfectly healthy memory nobody flagged");
    make_prune_eligible(&home, &doomed);
    make_prune_eligible(&home, &spared);

    let memories = data_dir(&home).join("memories");
    let live = |id: &str| {
        std::fs::read_dir(&memories)
            .expect("read memories dir")
            .filter_map(std::result::Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with(id))
            .count()
    };
    let path_of = |id: &str| -> Option<std::path::PathBuf> {
        std::fs::read_dir(&memories)
            .expect("read memories dir")
            .filter_map(std::result::Result::ok)
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(id))
            })
    };
    // The survivors' exact `{id}-{slug}.md` paths BEFORE the run. `live`
    // only counts files whose name starts with the id, so a rename (or a
    // re-save under a fresh slug) would otherwise still read as a survival;
    // comparing the post-run path against this snapshot does not.
    let spared_before = path_of(&spared).expect("the unlisted candidate is live before the run");
    let healthy_before = path_of(&healthy).expect("the non-candidate is live before the run");
    for (id, path) in [(&spared, &spared_before), (&healthy, &healthy_before)] {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        assert!(
            path.extension().is_some_and(|ext| ext == "md") && stem.starts_with(&format!("{id}-")),
            "seeded file must be named `{id}-<slug>.md`, got {path:?}"
        );
    }

    let paths = Paths::new(data_dir(&home));
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let req = maintenance::prune::Request {
        apply: true,
        limit: 50,
        offset: 0,
        // `healthy` is NOT a candidate — it must be ignored, not deleted.
        ids: vec![doomed.clone(), healthy.clone()],
    };
    maintenance::prune::run(&mut ctx, req).expect("prune run");

    assert_eq!(live(&doomed), 0, "the listed candidate must be pruned");
    assert_eq!(live(&spared), 1, "an unlisted candidate must survive");
    assert_eq!(live(&healthy), 1, "a non-candidate id must be ignored");
    assert_eq!(
        path_of(&spared).as_deref(),
        Some(spared_before.as_path()),
        "the unlisted candidate keeps its exact path"
    );
    assert_eq!(
        path_of(&healthy).as_deref(),
        Some(healthy_before.as_path()),
        "the non-candidate keeps its exact path"
    );
    assert!(
        spared_before.is_file() && healthy_before.is_file(),
        "both survivors stay exactly where they were: {spared_before:?}, {healthy_before:?}"
    );
    let trashed = std::fs::read_dir(memories.join(".trash"))
        .expect("read .trash")
        .count();
    assert_eq!(trashed, 1, "exactly one file may reach .trash/");
}

/// A malformed `ids` entry is a hard error naming the flag — the same
/// 8-hex validation `save --supersedes` applies — not a silently dropped
/// entry that would make `apply` act on the full candidate set instead.
#[test]
fn run_apply_rejects_a_malformed_id() {
    let home = TempDir::new().expect("tempdir");
    let id = save_memory(&home, "prune candidate guarded by id validation");
    make_prune_eligible(&home, &id);

    let paths = Paths::new(data_dir(&home));
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let req = maintenance::prune::Request {
        apply: true,
        limit: 50,
        offset: 0,
        ids: vec!["not-an-id".to_string()],
    };
    let err = maintenance::prune::run(&mut ctx, req).expect_err("malformed id must fail");
    assert!(err.to_string().contains("--ids"), "error was: {err}");

    let memories = data_dir(&home).join("memories");
    let trashed = std::fs::read_dir(memories.join(".trash"))
        .map(std::iter::Iterator::count)
        .unwrap_or_default();
    assert_eq!(trashed, 0, "a rejected request must delete nothing");
}

// ---------------------------------------------------------------------------
// #251: a pruned memory is a real deletion, so it replicates like one.
// ---------------------------------------------------------------------------

/// Feed positions as `"<op>:<entity_key>"`, oldest first.
fn feed_ops(conn: &rusqlite::Connection) -> Vec<String> {
    comemory::store::replica_read::page(conn, 0, 50, None)
        .expect("page")
        .into_iter()
        .map(|row| format!("{}:{}", row.op.as_str(), row.entity_key))
        .collect()
}

#[test]
fn apply_journals_a_tombstone_the_outbox_owes() {
    let home = TempDir::new().expect("tempdir");
    let id = save_memory(&home, "a prune candidate that must replicate its deletion");
    make_prune_eligible(&home, &id);

    let paths = Paths::new(data_dir(&home));
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let before = feed_ops(&conn);
    assert_eq!(
        before,
        vec![format!("upsert:{id}")],
        "the save journalled its upsert"
    );

    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let req = maintenance::prune::Request {
            apply: true,
            limit: 50,
            offset: 0,
            ids: Vec::new(),
        };
        maintenance::prune::run(&mut ctx, req).expect("prune apply");
    }

    assert_eq!(
        feed_ops(&conn),
        vec![format!("upsert:{id}"), format!("tombstone:{id}")],
        "a pruned memory replicates its deletion, or a peer re-offers it forever"
    );
    let pending = comemory::store::replica_outbox::pending(&conn, 10).expect("pending");
    let tombstones: Vec<_> = pending
        .iter()
        .filter(|row| row.op == comemory::store::replica_journal::ReplicaOp::Tombstone)
        .collect();
    assert_eq!(tombstones.len(), 1, "the tombstone is owed upstream");
    assert_eq!(tombstones[0].entity_key, id);
    assert_eq!(
        tombstones[0].payload_digest, None,
        "a tombstone names no payload"
    );
    assert!(
        comemory::store::memory_intent::outstanding(&conn)
            .expect("outstanding")
            .is_empty(),
        "the prune's delete finished, so it owes no reconciliation"
    );
}

#[test]
fn healing_a_half_deleted_row_journals_nothing_new() {
    let home = TempDir::new().expect("tempdir");
    let id = save_memory(&home, "a prune candidate whose markdown is already gone");
    make_prune_eligible(&home, &id);

    // The half-deleted state prune heals: the markdown is gone, the mirror row
    // is still live. Removing the file is exactly what a crash inside `delete`
    // between its move and its transaction leaves.
    let memories_dir = data_dir(&home).join("memories");
    for entry in std::fs::read_dir(&memories_dir).expect("read memories dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|ext| ext == "md") {
            std::fs::remove_file(&path).expect("remove markdown");
        }
    }

    let paths = Paths::new(data_dir(&home));
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let before = feed_ops(&conn);

    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let req = maintenance::prune::Request {
            apply: true,
            limit: 50,
            offset: 0,
            ids: Vec::new(),
        };
        maintenance::prune::run(&mut ctx, req).expect("prune apply");
    }

    assert_eq!(
        feed_ops(&conn),
        before,
        "a heal repairs a deletion that already happened; it is not a new one"
    );
}
