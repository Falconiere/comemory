#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
//! `comemory project export|import` (#342) through the real binary between
//! real data directories: a project with a row in every table round-trips
//! column for column with its receipt left behind (AC-1); a repeat is
//! `unchanged` (AC-2); a copy edited on one side is `skipped` with exit 0 and
//! both copies intact (AC-3); `project show` names the binding (AC-6); and
//! each refusal exits by its edge and writes nothing (AC-4, AC-9, AC-11).

use std::path::Path;

use comemory::store::project_table_shape::carried;
use rusqlite::Connection;
use rusqlite::types::Value as Sql;
use serde_json::Value;

#[path = "common/cli_bin.rs"]
mod cli_bin;

use cli_bin::CliHome;

const SEED: &str = include_str!("fixtures/projects/every_table_seed.sql");
const A: &str = "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f";

/// A migrated data directory, optionally loaded with the every-table seed.
fn home(seeded: bool) -> CliHome {
    let home = CliHome::new();
    home.run_ok(&["project", "list"]);
    if seeded {
        db(&home).execute_batch(SEED).unwrap();
    }
    home
}

fn db(home: &CliHome) -> Connection {
    Connection::open(home.data_dir().join("comemory.db")).unwrap()
}

/// Every row of `table`, sorted, rendered comparably.
fn table_rows(home: &CliHome, table: &str) -> Vec<String> {
    let conn = db(home);
    let mut statement = conn.prepare(&format!("SELECT * FROM \"{table}\"")).unwrap();
    let width = statement.column_count();
    let mut rows: Vec<String> = statement
        .query_map([], |r| {
            (0..width)
                .map(|i| r.get::<_, Sql>(i))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap()
        .map(|row| format!("{:?}", row.unwrap()))
        .collect();
    rows.sort();
    rows
}

/// Every project-owned table's rows, for "nothing was written".
fn project_state(home: &CliHome) -> Vec<Vec<String>> {
    comemory::store::schema_projects::PROJECT_TABLES
        .iter()
        .map(|t| table_rows(home, t))
        .collect()
}

/// Run `comemory <args>` expecting failure; `(exit code, stderr)`.
fn refused(home: &CliHome, args: &[&str]) -> (i32, String) {
    let out = home.bin().args(args).output().unwrap();
    assert!(!out.status.success(), "{args:?} unexpectedly succeeded");
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn arg(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// The first twelve hex characters of a digest, as the TTY lines print it.
fn short(digest: &Value) -> String {
    digest.as_str().unwrap().chars().take(12).collect()
}

/// Rows in every table of a bundle.
fn row_count(bundle: &Value) -> usize {
    bundle["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["rows"].as_array().unwrap().len())
        .sum()
}

#[test]
fn export_import_round_trip_between_two_data_directories() {
    let a = home(true);
    let scratch = tempfile::tempdir().unwrap();
    let file = scratch.path().join("alpha.json");
    let tty = a.run_ok(&["project", "export", A, "--output", arg(&file)]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "a bundle is owner-only");
    }
    let stdout_bundle: Value = serde_json::from_str(&a.run_ok(&["project", "export", A])).unwrap();
    let file_bundle: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(stdout_bundle, file_bundle);
    let (digest, rows) = (short(&file_bundle["digest"]), row_count(&file_bundle));
    assert_eq!(
        tty,
        format!(
            "exported {A} ({rows} rows, digest {digest}) to {}\n",
            file.display()
        )
    );
    let summary = a.run_json(&["project", "export", A, "--output", arg(&file)]);
    assert_eq!(
        summary,
        serde_json::json!({
            "projectId": A,
            "digest": file_bundle["digest"],
            "rows": rows,
            "output": file.display().to_string(),
        })
    );

    let b = home(false);
    let tty = b.run_ok(&["project", "import", arg(&file)]);
    assert_eq!(
        tty,
        format!("imported {A} ({rows} rows, digest {digest})\n")
    );
    for table in carried() {
        assert_eq!(
            table_rows(&b, &table.name),
            table_rows(&a, &table.name),
            "{} column for column",
            table.name
        );
    }
    assert!(table_rows(&b, "project_command_receipts").is_empty());
    assert_eq!(table_rows(&a, "project_command_receipts").len(), 1);

    let shown = b.run_json(&["project", "show", A]);
    let transfer = &shown["transfer"];
    assert_eq!(transfer["direction"], "imported");
    let absolute = std::path::absolute(&file).unwrap();
    assert_eq!(transfer["remote"], format!("file:{}", absolute.display()));
    assert_eq!(transfer["digest"], file_bundle["digest"]);
    assert!(transfer["transferredAt"].as_str().unwrap().ends_with('Z'));
    let tty = b.run_ok(&["project", "show", A]);
    assert!(tty.contains("transfer      imported from file:"), "{tty}");

    // AC-2: the same bundle again writes nothing.
    let before = project_state(&b);
    let again = b.run_json(&["project", "import", arg(&file)]);
    assert_eq!(again["outcome"], "unchanged");
    assert_eq!(again["digest"], file_bundle["digest"]);
    assert_eq!(again["localDigest"], again["digest"]);
    assert_eq!(again["rows"], 0);
    let tty = b.run_ok(&["project", "import", arg(&file)]);
    assert_eq!(
        tty,
        format!("unchanged {A}: this copy is identical (digest {digest})\n")
    );
    assert_eq!(project_state(&b), before);

    // AC-3: an edit in B makes A's bundle differ; nothing is overwritten.
    db(&b)
        .execute_batch("UPDATE project_work_items SET title = 'edited in B' WHERE id = 'w-1';")
        .unwrap();
    let before = project_state(&b);
    let skipped = b.run_json(&["project", "import", arg(&file)]);
    assert_eq!(skipped["outcome"], "skipped");
    assert_ne!(skipped["localDigest"], skipped["digest"]);
    assert_eq!(project_state(&b), before);
    let tty = b.run_ok(&["project", "import", arg(&file)]);
    let local = short(&skipped["localDigest"]);
    assert_eq!(
        tty,
        format!(
            "skipped {A}: this data directory holds a different copy \
             (local digest {local}, bundle digest {digest}); both are kept\n"
        )
    );
    let title = |home: &CliHome| -> String {
        db(home)
            .query_row(
                "SELECT title FROM project_work_items WHERE id = 'w-1'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(title(&b), "edited in B");
    assert_eq!(title(&a), "root");

    // Stdin is a source too, recorded as such.
    let c = home(false);
    let out = c
        .bin()
        .args(["--json", "project", "import", "-"])
        .write_stdin(std::fs::read(&file).unwrap())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let shown = c.run_json(&["project", "show", A]);
    assert_eq!(shown["transfer"]["remote"], "stdin");

    // A local mutation of a bound project says the change stays here (AC-7):
    // JSON carries the warning, stderr prints it, and the change is made.
    let out = c
        .bin()
        .args(["--json", "project", "restore", A, "--expected-version", "7"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let restored: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        restored["warnings"],
        serde_json::json!([{
            "code": "local_only",
            "projectId": A,
            "direction": "imported",
            "remote": "stdin",
            "message": "This project was imported from stdin; this change stays in this data \
                        directory and will not reach stdin",
        }])
    );
    assert!(restored["project"]["archivedAt"].is_null());
    assert_eq!(
        String::from_utf8(out.stderr).unwrap(),
        "warning: This project was imported from stdin; this change stays in this data \
         directory and will not reach stdin\n"
    );

    // `--remote` names the other side instead of the file.
    let d = home(false);
    d.run_ok(&["project", "import", arg(&file), "--remote", "ws-123"]);
    let shown = d.run_json(&["project", "show", A]);
    assert_eq!(shown["transfer"]["remote"], "ws-123");
}

/// An unreadable bundle or an unwritable `--output` is an I/O error (exit
/// 74), and neither writes to the store or leaves a partial file.
#[test]
fn unreadable_bundles_and_unwritable_outputs_exit_74() {
    let a = home(true);
    let scratch = tempfile::tempdir().unwrap();
    let missing_dir = scratch.path().join("no-such-dir").join("alpha.json");
    let (code, stderr) = refused(&a, &["project", "export", A, "--output", arg(&missing_dir)]);
    assert_eq!(code, 74, "{stderr}");
    assert!(!missing_dir.exists());
    assert!(!missing_dir.parent().unwrap().exists());

    let b = home(false);
    let before = project_state(&b);
    let absent = scratch.path().join("absent.json");
    let (code, stderr) = refused(&b, &["project", "import", arg(&absent)]);
    assert_eq!(code, 74, "{stderr}");
    assert_eq!(project_state(&b), before);
}

#[test]
fn import_refusals_exit_by_edge_and_write_nothing() {
    let a = home(true);
    let scratch = tempfile::tempdir().unwrap();
    let file = scratch.path().join("alpha.json");
    a.run_ok(&["project", "export", A, "--output", arg(&file)]);

    // AC-4: another project already holds the key prefix.
    let c = home(false);
    c.run_ok(&[
        "project",
        "create",
        "--name",
        "Other",
        "--key-prefix",
        "ALP",
        "--outcome",
        "o",
    ]);
    let before = project_state(&c);
    let (code, stderr) = refused(&c, &["project", "import", arg(&file)]);
    assert_eq!(code, 65, "{stderr}");
    assert!(
        stderr.contains("keyPrefix is already used in this workspace"),
        "{stderr}"
    );
    assert_eq!(project_state(&c), before);

    // AC-9: a row edited after export no longer matches the digest.
    let mut bundle: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    let items = bundle["tables"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|t| t["table"] == "project_work_items")
        .unwrap();
    items["rows"][0][6] = Value::from("not what was digested");
    let tampered = scratch.path().join("tampered.json");
    std::fs::write(&tampered, serde_json::to_vec(&bundle).unwrap()).unwrap();
    let d = home(false);
    let (code, stderr) = refused(&d, &["project", "import", arg(&tampered)]);
    assert_eq!(code, 64, "{stderr}");
    assert!(
        stderr.contains("bundle digest does not match its rows"),
        "{stderr}"
    );
    let not_json = scratch.path().join("not.json");
    std::fs::write(&not_json, b"not json").unwrap();
    let (code, stderr) = refused(&d, &["project", "import", arg(&not_json)]);
    assert_eq!(code, 64, "{stderr}");
    assert!(stderr.contains("bundle is not JSON"), "{stderr}");
    assert!(project_state(&d).iter().all(Vec::is_empty));
}

#[test]
fn export_refuses_unknown_and_malformed_ids() {
    let a = home(false);
    let unknown = "00000000-0000-4000-8000-000000000000";
    let (code, stderr) = refused(&a, &["project", "export", unknown]);
    assert_eq!(code, 64, "{stderr}");
    assert!(stderr.contains("Project not found"), "{stderr}");
    let (code, stderr) = refused(&a, &["project", "export", "nope"]);
    assert_eq!(code, 64, "{stderr}");
    assert!(stderr.contains("projectId is invalid"), "{stderr}");
}
