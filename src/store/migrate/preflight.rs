//! Migration preflight: refuse a database written by a newer comemory, and
//! take a pre-upgrade snapshot before any pending migration that would
//! destroy data.
//!
//! Runs once, from [`crate::store::connection::open`], **after** the
//! identifier tokenizer is registered and **before** [`super::run`] — that
//! ordering is load-bearing, not incidental: [`snapshot`](super::backup::snapshot)
//! (`VACUUM INTO`) prepares the source schema, which instantiates every
//! FTS5 virtual table, and any v4+ database carries `tokenize =
//! 'identifier'` columns (`memory_fts`, `code_fts`, and later `edge_fts` /
//! `document_fts`). A preflight run above the tokenizer registration fails
//! with "unknown tokenizer: identifier" on exactly the databases it exists
//! to protect.
//!
//! Gates on the **applied-marker set** recorded in `schema_meta`, not the
//! `version` string: `set_version` is written once, at the very end of the
//! migration chain, so it cannot distinguish "fully migrated" from
//! "crashed after the last migration, before the version write". The
//! comparison is against the union of every [`super::list::Migration`]'s
//! `markers` — never against [`super::list::MIGRATIONS`]' keys directly,
//! since `0001_schema_meta` records no marker and `0004`/`0005` each write
//! a second, non-migration marker for their simhash post-pass; comparing
//! against keys would refuse every existing v4+ database on Earth.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::list::{self, Class};
use crate::config::{env, paths::Paths};
use crate::prelude::*;
use crate::source::lock::FileLock;

/// Guard and, if needed, snapshot the database at `db_path` (already open
/// as `conn`) before [`super::run`] touches it. See the module doc for the
/// four branches this implements.
pub(crate) fn preflight(conn: &Connection, db_path: &Path) -> Result<()> {
    if !schema_meta_exists(conn)? {
        if !has_any_table(conn)? {
            // A brand-new, empty file — nothing to guard or snapshot.
            return Ok(());
        }
        // A legacy database that predates `schema_meta` entirely: nothing
        // is recorded as applied, so every migration is pending.
        return snapshot_before_pending(conn, db_path, &BTreeSet::new());
    }

    let applied = applied_keys(conn)?;
    let expected = expected_markers();

    if let Some(unknown) = applied.iter().find(|k| !expected.contains(k.as_str())) {
        return Err(forward_compat_error(db_path, unknown));
    }
    if applied.len() == expected.len() {
        // `applied` was just shown to be a subset of `expected` (the find
        // above found no counterexample); equal cardinality then means the
        // sets are equal — the fully-migrated state every read-only
        // command hits, at zero extra I/O.
        return Ok(());
    }

    snapshot_before_pending(conn, db_path, &applied)
}

/// The applied-migration keys recorded in `schema_meta` — every row whose
/// key looks like a migration marker (`0001_schema_meta`,
/// `0004_simhash_backfill`, ...). Only called once `schema_meta` is known
/// to exist. `pub(crate)`: [`crate::api::doctor`] reuses this against its
/// own read-only fallback connection rather than re-querying by hand.
pub(crate) fn applied_keys(conn: &Connection) -> Result<BTreeSet<String>> {
    let mut stmt = conn.prepare("SELECT key FROM schema_meta WHERE key LIKE '0%'")?;
    let keys = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<BTreeSet<String>, rusqlite::Error>>()?;
    Ok(keys)
}

/// Union of every [`list::Migration::markers`] entry across
/// [`list::MIGRATIONS`] — every `schema_meta` key a fully-migrated database
/// holds. `pub(crate)`: [`crate::api::doctor`] reuses this to compute the
/// same unknown-key set its read-only fallback reports.
pub(crate) fn expected_markers() -> BTreeSet<&'static str> {
    list::MIGRATIONS
        .iter()
        .flat_map(|m| m.markers.iter().copied())
        .collect()
}

/// True when `migration` still has pending work: at least one of its
/// declared `markers` is absent from `applied`. `0001_schema_meta` (empty
/// `markers`) is vacuously never pending — its own SQL is
/// `CREATE TABLE IF NOT EXISTS` and idempotent regardless.
fn is_pending(migration: &list::Migration, applied: &BTreeSet<String>) -> bool {
    !migration
        .markers
        .iter()
        .all(|marker| applied.contains(*marker))
}

/// True when `schema_meta` exists as a table.
fn schema_meta_exists(conn: &Connection) -> Result<bool> {
    table_exists(conn, "schema_meta")
}

/// True when the database holds at least one table of any name —
/// distinguishes a brand-new empty file (fresh, no snapshot needed) from a
/// legacy database that predates `schema_meta` (snapshot warranted).
fn has_any_table(conn: &Connection) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table'",
        [],
        |row| row.get(0),
    )?;
    Ok(n > 0)
}

/// True when a table named `name` exists.
fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |row| row.get(0),
    )?;
    Ok(n > 0)
}

/// Build the forward-compat refusal: `applied` holds a key `unknown` to
/// this build, meaning the database was written by a newer comemory.
/// `schema_meta` is left untouched — this returns before anything is
/// written.
fn forward_compat_error(db_path: &Path, unknown: &str) -> Error {
    Error::Migration(format!(
        "database at {} was written by a newer comemory (applied migration {unknown} is unknown \
         to this build, which supports through {highest}). Upgrade comemory, or point \
         COMEMORY_DATA_DIR elsewhere.",
        db_path.display(),
        highest = highest_key(),
    ))
}

/// The last (highest) migration key this build knows how to apply, named
/// in the forward-compat error.
fn highest_key() -> &'static str {
    list::MIGRATIONS.last().map_or("", |m| m.key)
}

/// `applied ⊊ expected`: at least one migration is still pending. Snapshot
/// before it runs unless [`env::skip_migration_backup`] opts out.
/// Mandatory (a failure refuses the upgrade) when any pending migration is
/// [`Class::Destructive`]; advisory (a failure only warns) otherwise.
///
/// "Pending" is computed by filtering [`is_pending`] over the whole list,
/// not by slicing from the first pending entry onward: in the ordinary
/// crash-recovery case those two are the same set (migrations apply
/// strictly in order, so nothing after the first gap can be done either),
/// but computing the real filtered set keeps the mandatory/advisory split
/// correct even if a marker were ever missing out of sequence.
fn snapshot_before_pending(
    conn: &Connection,
    db_path: &Path,
    applied: &BTreeSet<String>,
) -> Result<()> {
    if env::skip_migration_backup() {
        return Ok(());
    }

    let pending_from = list::MIGRATIONS
        .iter()
        .position(|m| is_pending(m, applied))
        .unwrap_or(list::MIGRATIONS.len());
    let mandatory = list::MIGRATIONS
        .iter()
        .filter(|m| is_pending(m, applied))
        .any(|m| m.class == Class::Destructive);

    let dest = backup_dest(db_path, pending_from as u32);
    notify_snapshot_starting(&dest);

    match take_snapshot(conn, db_path, &dest) {
        Ok(()) => Ok(()),
        Err(e) if mandatory => Err(Error::Migration(format!(
            "pre-migration snapshot to {} failed and this upgrade includes a destructive \
             migration, so it cannot proceed safely: {e}",
            dest.display()
        ))),
        Err(e) => {
            warn_snapshot_failed(&e);
            Ok(())
        }
    }
}

/// Prune older snapshots in `db_path`'s namespace, then `VACUUM INTO`
/// `dest` — both under the migration [`FileLock`], since SQLite will not
/// serialize `VACUUM INTO` on its own.
fn take_snapshot(conn: &Connection, db_path: &Path, dest: &Path) -> Result<()> {
    let dir = dir_of(db_path);
    let lock_path = Paths::new(dir).migration_lock_file();
    let _guard = FileLock::acquire(&lock_path, "migration")?;
    super::backup::prune(dir, &backup_prefix(db_path))?;
    super::backup::snapshot(conn, dest)
}

/// Pre-migration snapshot destination, derived from the actual database
/// **file name** — not `Paths::db_path()` — since `connection::open` is
/// also called on `comemory.db.rebuild.tmp` (`api::rebuild::run`), and
/// resolving through `Paths` there would collide the tmp database's backup
/// namespace with the live one in the same directory. `from` is the count
/// of fully-applied migrations preceding the first pending one, i.e. the
/// version being left.
fn backup_dest(db_path: &Path, from: u32) -> PathBuf {
    dir_of(db_path).join(format!("{}.pre-v{from}.bak", file_name_of(db_path)))
}

/// The `prune` prefix matching every snapshot [`backup_dest`] can produce
/// for `db_path`, regardless of `from` — prefix-scoped so this database
/// file's migration budget cannot evict a different database file's (e.g.
/// the live DB's and the rebuild tmp DB's, which share a directory).
fn backup_prefix(db_path: &Path) -> String {
    format!("{}.pre-v", file_name_of(db_path))
}

/// `db_path`'s parent directory, or `.` if it has none (a bare relative
/// filename).
fn dir_of(db_path: &Path) -> &Path {
    db_path.parent().unwrap_or_else(|| Path::new("."))
}

/// `db_path`'s file name as UTF-8 (lossy), or empty if it has none.
fn file_name_of(db_path: &Path) -> Cow<'_, str> {
    db_path
        .file_name()
        .map_or(Cow::Borrowed(""), std::ffi::OsStr::to_string_lossy)
}

/// One line to stderr before `VACUUM INTO` starts, so a long pause on a
/// large database is explained rather than mysterious. `tracing` is not
/// usable here: `main.rs` initializes `EnvFilter::from_default_env()`, and
/// with `RUST_LOG` unset no event is emitted at all — so a diagnostic that
/// must always be visible goes to stderr directly, the same mechanism
/// `main.rs::report` uses. Never stdout, so `--json` output stays clean.
fn notify_snapshot_starting(dest: &Path) {
    let mut sink = std::io::stderr().lock();
    let _ = writeln!(
        sink,
        "comemory: snapshotting database to {} before migrating (set \
         COMEMORY_SKIP_MIGRATION_BACKUP=1 to skip)",
        dest.display()
    );
}

/// One line to stderr when an advisory (additive-only) snapshot failed and
/// the upgrade is proceeding anyway — see [`notify_snapshot_starting`] for
/// why stderr rather than `tracing`.
fn warn_snapshot_failed(err: &Error) {
    let mut sink = std::io::stderr().lock();
    let _ = writeln!(
        sink,
        "comemory: warning: pre-migration snapshot failed ({err}); continuing without one since \
         no pending migration is destructive"
    );
}

#[cfg(test)]
#[path = "tests/preflight.rs"]
mod tests;
