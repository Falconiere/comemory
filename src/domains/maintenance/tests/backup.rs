#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/maintenance/backup.rs` and its restore,
//! swap and merge siblings (#256, B-4): real data directories, real migrated
//! databases, memories saved and erased through the production cores, real
//! `VACUUM INTO` backups, a real SQLite write lock held across the swap, and
//! two real database files with different page sizes.

use std::fs;
use std::path::{Path, PathBuf};

use comemory::domains::maintenance::backup::{
    self, DB_FILE, DESCRIPTOR_FILE, Descriptor, MEMORIES_DIR, RestoreRequest, Restored,
};
use comemory::domains::maintenance::erase::{self, Request};
use comemory::domains::maintenance::rebuild;
use comemory::domains::sync::replica::identity::{
    self, ERASURES_KEY, Ensured, MANIFEST_FILE, REPLACED, RESTORED,
};
use comemory::domains::sync::replica::restore_state;
use comemory::prelude::Error;
use comemory::store::{connection, replica_device};
use comemory::utilities::context::Ctx;
use rusqlite::Connection;
use tempfile::TempDir;

use crate::domains::sync::replica::test_support::{BODY, Home};

/// A word no query ever uses: finding it after a restore means the erase
/// the backup predates was not merged.
const TOKEN: &str = "qzvxBackupToken0925";

fn token_body() -> String {
    format!("{BODY} It also records {TOKEN}, which nothing ever searches for.")
}

/// Every file under `dir` whose bytes contain `needle`, skipping `except`.
fn files_holding(dir: &Path, needle: &str, except: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if except.contains(&path) {
            continue;
        }
        if path.is_dir() {
            found.extend(files_holding(&path, needle, except));
        } else if fs::read(&path)
            .unwrap_or_default()
            .windows(needle.len())
            .any(|w| w == needle.as_bytes())
        {
            found.push(path);
        }
    }
    found
}

fn erase_memory(home: &mut Home, id: &str) {
    let mut ctx = home.ctx();
    erase::run(
        &mut ctx,
        Request {
            memory: Some(id.to_string()),
            document: None,
        },
    )
    .expect("erase");
}

fn admit(home: &mut Home) -> comemory::prelude::Result<Ensured> {
    restore_state::admit(&home.paths, &home.cfg, &mut home.conn)
}

fn state(home: &Home) -> Option<String> {
    restore_state::read(&home.conn).expect("state")
}

fn stamped(conn: &Connection) -> Option<String> {
    conn.query_row(
        "SELECT value FROM schema_meta WHERE key = ?1",
        [ERASURES_KEY],
        |r| r.get(0),
    )
    .ok()
}

fn restore_of(home: &Home, dir: &Path) -> comemory::prelude::Result<Restored> {
    backup::restore(
        &home.paths,
        &home.cfg,
        &RestoreRequest {
            dir: dir.to_path_buf(),
            erasure_manifest: None,
        },
    )
}

fn live_memory_ids(conn: &Connection) -> Vec<String> {
    let mut statement = conn
        .prepare("SELECT id FROM memories WHERE deleted_at IS NULL ORDER BY id")
        .expect("prepare");
    statement
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect")
}

/// A home holding one memory carrying [`TOKEN`], its identity written, and a
/// backup of it in a directory of its own. Returns the home, the memory id,
/// the backup directory's owner and the backup directory.
fn backed_up_home() -> (Home, String, TempDir, PathBuf) {
    let mut home = Home::new();
    let id = home.save(&token_body(), &["backup"]);
    assert_eq!(admit(&mut home).expect("admit"), Ensured::Created);
    let owner = tempfile::tempdir().expect("backup root");
    let dir = owner.path().join("snapshot");
    backup::create(&home.paths, &home.cfg, Some(dir.clone())).expect("create");
    (home, id, owner, dir)
}

#[test]
fn create_writes_the_database_the_markdown_and_the_descriptor() {
    let mut home = Home::new();
    let kept = home.save(BODY, &["backup"]);
    let trashed = home.save("A memory the backup keeps in its trash.", &["backup"]);
    home.delete(&trashed);

    let created = backup::create(&home.paths, &home.cfg, None).expect("create");

    let dir = PathBuf::from(&created.dir);
    assert_eq!(
        dir.parent(),
        Some(home.paths.data_dir().join(backup::BACKUPS_DIR).as_path()),
        "the default lands under <data_dir>/backups/"
    );
    assert_eq!(created.memory_files, 2, "live and trashed markdown");
    let descriptor: Descriptor =
        serde_json::from_slice(&fs::read(dir.join(DESCRIPTOR_FILE)).expect("backup.json"))
            .expect("parses");
    assert_eq!(descriptor, created.descriptor);
    assert_eq!(descriptor.epoch, home.epoch());
    assert_eq!(descriptor.binary_version, env!("CARGO_PKG_VERSION"));
    assert!(
        descriptor
            .schema_markers
            .iter()
            .any(|m| m == "0029_replica_recovery"),
        "{:?}",
        descriptor.schema_markers
    );
    let copy = Connection::open(dir.join(DB_FILE)).expect("open the copy");
    assert_eq!(live_memory_ids(&copy), vec![kept.clone()]);
    let trash = dir.join(MEMORIES_DIR).join(".trash");
    assert_eq!(fs::read_dir(&trash).expect("trash copied").count(), 1);
    assert!(
        fs::read_dir(dir.join(MEMORIES_DIR))
            .expect("memories copied")
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with(&kept))
    );

    let again = backup::create(&home.paths, &home.cfg, Some(dir.clone()));
    assert!(
        matches!(again, Err(Error::Conflict(_))),
        "a directory already holding a backup is refused: {again:?}"
    );
}

#[test]
fn restore_rotates_the_epoch_keeps_the_device_and_merges_the_erase() {
    let (mut home, id, _owner, dir) = backed_up_home();
    let (before_epoch, device) = (
        home.epoch(),
        replica_device::id(&home.conn).expect("device"),
    );
    erase_memory(&mut home, &id);

    let restored = restore_of(&home, &dir).expect("restore");

    assert_ne!(restored.epoch, before_epoch, "a restore is a new stream");
    assert_eq!(home.epoch(), restored.epoch, "the live file carries it");
    assert_eq!(restored.device_id, device, "the device is kept");
    assert_eq!(restored.restore_state, None);
    assert_eq!(restored.erasures_merged, 1);
    assert!(!restored.resumed);
    let identity = identity::read(&home.paths)
        .expect("read")
        .expect("identity");
    assert_eq!(identity.epoch, restored.epoch);
    assert_eq!(
        identity.epochs.last().map(|e| e.reason.as_str()),
        Some(RESTORED)
    );
    assert_eq!(stamped(&home.conn).as_deref(), Some("1"));
    assert!(
        live_memory_ids(&home.conn).is_empty(),
        "the erased memory is not brought back"
    );
    let kept: Vec<PathBuf> = restored.pre_restore.iter().map(PathBuf::from).collect();
    assert_eq!(kept.len(), 2, "{kept:?}");
    assert_eq!(
        files_holding(home.paths.data_dir(), TOKEN, &kept),
        Vec::<PathBuf>::new(),
        "the token survives the merge"
    );
    for leftover in [
        "comemory.db.restore.tmp",
        "memories.restore",
        "restore.pending",
    ] {
        assert!(
            !home.paths.data_dir().join(leftover).exists(),
            "{leftover} is cleaned up"
        );
    }
    assert_eq!(admit(&mut home).expect("admit"), Ensured::Current);
}

#[test]
fn restore_without_a_manifest_is_local_only_until_merged() {
    let (mut home, id, _owner, dir) = backed_up_home();
    erase_memory(&mut home, &id);
    let manifest = identity::file(&home.paths, MANIFEST_FILE);
    let moved = home.paths.data_dir().join("moved.jsonl");
    fs::rename(&manifest, &moved).expect("move the manifest away");

    let restored = restore_of(&home, &dir).expect("restore completes");

    assert_eq!(restored.restore_state.as_deref(), Some("erasure_unknown"));
    assert_eq!(
        live_memory_ids(&home.conn),
        vec![id.clone()],
        "local-only: the erase is unknown here"
    );
    assert!(matches!(admit(&mut home), Err(Error::RestoreUnverified(_))));

    // A rebuild carries the state: it is a `replica_` key.
    {
        let mut ctx = Ctx::lazy(&home.paths, &home.cfg);
        rebuild::run(&mut ctx, rebuild::Request {}).expect("rebuild");
    }
    assert_eq!(state(&home).as_deref(), Some("erasure_unknown"));
    assert!(matches!(admit(&mut home), Err(Error::RestoreUnverified(_))));

    let merged = backup::merge_erasures(&home.paths, &home.cfg, &moved).expect("merge");

    assert_eq!(merged.cleared.as_deref(), Some("erasure_unknown"));
    assert_eq!((merged.lines, merged.entities_erased), (1, 1));
    assert!(manifest.exists(), "the manifest is back in place");
    assert_eq!(state(&home), None);
    assert!(
        live_memory_ids(&home.conn).is_empty(),
        "merged: erased again"
    );
    assert_eq!(admit(&mut home).expect("admit"), Ensured::Current);
}

#[test]
fn merge_refuses_a_manifest_that_is_not_established() {
    let (mut home, id, _owner, _dir) = backed_up_home();
    erase_memory(&mut home, &id);
    let manifest = identity::file(&home.paths, MANIFEST_FILE);
    let bytes = fs::read(&manifest).expect("bytes");
    let torn = home.paths.data_dir().join("torn.jsonl");
    fs::write(&torn, &bytes[..bytes.len() - 10]).expect("tear it");

    let refused = backup::merge_erasures(&home.paths, &home.cfg, &torn);

    assert!(matches!(refused, Err(Error::Conflict(_))), "{refused:?}");
    assert_eq!(
        fs::read(&manifest).expect("bytes"),
        bytes,
        "nothing replaced"
    );
    let missing =
        backup::merge_erasures(&home.paths, &home.cfg, &home.paths.data_dir().join("none"));
    assert!(matches!(missing, Err(Error::Usage(_))), "{missing:?}");
}

#[test]
fn a_swap_held_off_by_a_writer_is_finished_by_the_rerun() {
    let (mut home, id, _owner, dir) = backed_up_home();
    erase_memory(&mut home, &id);
    home.cfg.sync.pause_wait = "1s".to_string();
    let blocker = Connection::open(home.paths.db_path()).expect("blocker");
    blocker
        .execute_batch(
            "BEGIN IMMEDIATE; UPDATE schema_meta SET value = value WHERE key = 'version';",
        )
        .expect("hold the write lock");

    let held = restore_of(&home, &dir);

    assert!(matches!(held, Err(Error::Busy(_))), "{held:?}");
    let pending = restore_state::pending_path(&home.paths);
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(&pending).expect("restore.pending stays")).expect("json");
    assert_eq!(record["phase"], "swapping");
    assert!(
        !home.paths.data_dir().join("memories.restore").exists(),
        "the markdown was already swapped"
    );
    assert!(
        matches!(admit(&mut home), Err(Error::RestoreUnverified(_))),
        "sync refuses mid-swap"
    );
    blocker.execute_batch("ROLLBACK;").expect("release");

    let finished = restore_of(&home, &dir).expect("the rerun finishes");

    assert!(finished.resumed);
    assert_eq!(home.epoch(), finished.epoch);
    assert!(!pending.exists());
    assert!(live_memory_ids(&home.conn).is_empty());
    assert_eq!(admit(&mut home).expect("admit"), Ensured::Current);
}

#[test]
fn a_snapshot_with_another_page_size_is_refused_naming_both() {
    let (home, _id, owner, _dir) = backed_up_home();
    let other = owner.path().join("other-pages");
    fs::create_dir_all(&other).expect("dir");
    {
        let conn = Connection::open(other.join(DB_FILE)).expect("open");
        conn.pragma_update(None, "page_size", 8192_i64)
            .expect("page size");
        conn.execute_batch("CREATE TABLE t(v TEXT); INSERT INTO t VALUES ('x');")
            .expect("seed");
    }
    let before = home.epoch();

    let refused = restore_of(&home, &other);

    let Err(Error::Unsupported(why)) = refused else {
        panic!("a page-size mismatch is refused: {refused:?}");
    };
    assert!(why.contains("8192") && why.contains("4096"), "{why}");
    assert!(
        why.contains(&other.join(DB_FILE).display().to_string())
            && why.contains(&home.paths.db_path().display().to_string()),
        "{why}"
    );
    assert_eq!(home.epoch(), before, "nothing changed");
    assert!(!restore_state::pending_path(&home.paths).exists());
}

/// Close `home`'s connection, copy `bak` over `comemory.db` the way an
/// operator would by hand, and open it again.
fn copy_over(home: &mut Home, bak: &Path) {
    let open = std::mem::replace(
        &mut home.conn,
        Connection::open_in_memory().expect("placeholder"),
    );
    open.close().expect("close");
    let db = home.paths.db_path();
    for side in ["-wal", "-shm"] {
        let _ = fs::remove_file(PathBuf::from(format!("{}{side}", db.display())));
    }
    fs::copy(bak, &db).expect("copy the .bak over comemory.db");
    home.conn = connection::open(&db).expect("reopen");
}

#[test]
fn a_replaced_database_without_its_manifest_stays_refused_until_merged() {
    let (mut home, id, _owner, dir) = backed_up_home();
    erase_memory(&mut home, &id);
    let manifest = identity::file(&home.paths, MANIFEST_FILE);
    let moved = home.paths.data_dir().join("moved.jsonl");
    fs::rename(&manifest, &moved).expect("move the manifest away");
    copy_over(&mut home, &dir.join(DB_FILE));

    let first = admit(&mut home);

    assert!(
        matches!(first, Err(Error::RestoreUnverified(_))),
        "the very read that found the erasures unknown is refused: {first:?}"
    );
    let identity = identity::read(&home.paths)
        .expect("read")
        .expect("identity");
    assert_eq!(
        identity.epochs.last().map(|e| e.reason.as_str()),
        Some(REPLACED)
    );
    assert_eq!(home.epoch(), identity.epoch, "re-epoched once");
    assert_eq!(state(&home).as_deref(), Some("erasure_unknown"));
    assert!(
        matches!(admit(&mut home), Err(Error::RestoreUnverified(_))),
        "and every later one, until the merge"
    );

    backup::merge_erasures(&home.paths, &home.cfg, &moved).expect("merge");

    assert_eq!(state(&home), None);
    assert_eq!(admit(&mut home).expect("admit"), Ensured::Current);
    assert!(live_memory_ids(&home.conn).is_empty());
}
