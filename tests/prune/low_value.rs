use qwick_memory::config::paths::Paths;
use qwick_memory::memory::{Kind, MemoryStore};
use qwick_memory::prune::low_value;
use qwick_memory::stats::feedback::Feedback;
use qwick_memory::stats::sqlite::StatsDb;

use super::common;

#[test]
fn fresh_memory_is_not_low_value() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    // quality=1 (< below_quality=2), zero feedback — but created ≈ now, so the
    // 180-day age gate filters it out.
    let _ = store.save("body", Kind::Note, "r", &[], "a", 1).unwrap();
    let ids = low_value::detect(&paths, 2, 180).unwrap();
    assert!(
        ids.is_empty(),
        "newly created memory should not be marked low-value; got {ids:?}",
    );
}

#[test]
fn high_quality_memory_is_never_flagged() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    // quality=5 is well above the below_quality=2 cutoff. Even with the age
    // gate fully open (unused_since_days=0), it must not be flagged.
    let _ = store
        .save("high quality", Kind::Decision, "r", &[], "a", 5)
        .unwrap();
    let ids = low_value::detect(&paths, 2, 0).unwrap();
    assert!(
        ids.is_empty(),
        "quality=5 must never be flagged as low-value, got {ids:?}",
    );
}

#[test]
fn used_memory_is_never_flagged() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let rec = store
        .save("low quality but used", Kind::Note, "r", &[], "a", 1)
        .unwrap();

    // Mark the memory as used so the feedback gate excludes it even though
    // quality and age would otherwise qualify.
    let mut db = StatsDb::open(paths.stats_db()).unwrap();
    Feedback::new(&mut db)
        .record_used(&rec.frontmatter.id)
        .unwrap();
    drop(db);

    // unused_since_days=0 opens the age gate as wide as possible; if the
    // feedback gate is honored, the result must still be empty.
    let ids = low_value::detect(&paths, 2, 0).unwrap();
    assert!(
        !ids.contains(&rec.frontmatter.id),
        "used memory must not be flagged as low-value; got {ids:?}",
    );
}
