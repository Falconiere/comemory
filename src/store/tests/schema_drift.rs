#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The generate loop against the SHIPPED journal and baseline snapshot —
//! real files copied into a tempdir so `run_generate` can write beside them.
//!
//! Proves two things a hand-inspected snapshot cannot: that the shipped
//! `0016_v16_sync.snapshot.json` describes `registry()` exactly (a struct
//! edited without `just migration <name>` makes `run_generate` emit a file
//! here), and that what `run_generate` emits for a real change is SQL the
//! runtime runner applies as-is onto a real database — `execute_batch` over
//! a file whose statements are separated by `--> statement-breakpoint`
//! comment lines.
//!
//! All fns are prefixed `schema_drift_` so
//! `cargo nextest run -E 'test(schema_drift)'` selects exactly this suite.

use std::fs;
use std::path::{Path, PathBuf};

use comemory::store::connection;
use comemory::store::schema::registry;
use tempfile::tempdir;
use toolu_orm::core::column::{ColumnDef, ColumnType};
use toolu_orm::core::dialect::Dialect;
use toolu_orm::core::schema::SchemaRegistry;
use toolu_orm_cli::generate::run_generate;

const SHIPPED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");

/// A tempdir holding copies of the shipped `_journal.json` and the newest
/// snapshot — everything `run_generate` reads.
fn shipped_copy() -> (tempfile::TempDir, PathBuf) {
    let dir = tempdir().expect("tempdir");
    for name in ["_journal.json", "0016_v16_sync.snapshot.json"] {
        fs::copy(Path::new(SHIPPED).join(name), dir.path().join(name))
            .unwrap_or_else(|e| panic!("copy {name}: {e}"));
    }
    let path = dir.path().to_path_buf();
    (dir, path)
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn schema_drift_registry_matches_shipped_snapshot() {
    let (_guard, dir) = shipped_copy();
    let before = entries(&dir);

    let written = run_generate(
        &registry(),
        dir.to_str().unwrap(),
        "drift_probe",
        Dialect::Sqlite,
    )
    .expect("run_generate over the shipped journal + snapshot");

    assert_eq!(
        written, None,
        "the registry drifted from migrations/0016_v16_sync.snapshot.json — run `just migration \
         <name>` and wire the result, or `just migration-adopt` if the change is a restatement"
    );
    assert_eq!(entries(&dir), before, "a no-op generate must write nothing");
}

/// `registry()` with one nullable `INTEGER probe` column appended to
/// `feedback` — the smallest real change the generate loop can see.
fn registry_with_feedback_probe() -> SchemaRegistry {
    let mut tables = registry().tables().to_vec();
    let feedback = tables
        .iter_mut()
        .find(|t| t.name == "feedback")
        .expect("feedback is declared");
    feedback.columns.push(ColumnDef {
        name: "probe".to_owned(),
        column_type: ColumnType::Integer,
        primary_key: false,
        not_null: false,
        default: None,
        unique: false,
        references: None,
        on_delete: None,
        on_update: None,
        check: None,
        unindexed: false,
    });
    SchemaRegistry::from_tables(tables)
}

#[test]
fn schema_drift_generate_emits_an_applicable_alter_for_a_new_column() {
    let (_guard, dir) = shipped_copy();

    let written = run_generate(
        &registry_with_feedback_probe(),
        dir.to_str().unwrap(),
        "drift_probe",
        Dialect::Sqlite,
    )
    .expect("run_generate");
    assert_eq!(written.as_deref(), Some("0017_drift_probe.sql"));

    let sql = fs::read_to_string(dir.join("0017_drift_probe.sql")).unwrap();
    assert!(
        sql.contains("ALTER TABLE \"feedback\" ADD COLUMN \"probe\" INTEGER"),
        "generated SQL:\n{sql}"
    );
    assert!(
        dir.join("0017_drift_probe.snapshot.json").exists(),
        "a snapshot accompanies the migration"
    );
    let journal = fs::read_to_string(dir.join("_journal.json")).unwrap();
    assert!(
        journal.contains("0017_drift_probe.sql"),
        "journal gained the entry"
    );

    // The runtime runner applies generated SQL verbatim: breakpoint lines are
    // SQL comments to `execute_batch`.
    let db_dir = tempdir().expect("tempdir");
    let conn = connection::open(db_dir.path().join("comemory.db")).expect("open");
    conn.execute_batch(&sql)
        .unwrap_or_else(|e| panic!("generated migration must apply: {e}\n{sql}"));
    let has_probe: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_table_info('feedback') WHERE name = 'probe'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        has_probe, 1,
        "feedback.probe exists after applying the generated file"
    );
}
