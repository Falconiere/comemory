#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! [`comemory::domains::memories::recover::reconcile`] against real data
//! directories left in each state a killed memory write produces: the
//! markdown on disk with no mirror row, the rename that never landed, the
//! mirror committed but the journal not, the `.trash/` move without its
//! delete, and the delete that never moved the file.
//!
//! Every fixture is built by running the real production writers and then
//! removing exactly one side, so no state here is one the engine could not
//! actually reach.

use comemory::config::{Config, Paths};
use comemory::domains::memories::recover::{self, Report};
use comemory::domains::memories::{
    self, Kind, MemoryStore, References, Relations, SaveParams, id,
};
use comemory::store::memory_intent::{self, Intent, IntentKind};
use comemory::store::{connection, replica_outbox, replica_read};
use comemory::utilities::context::Ctx;
use rusqlite::Connection;
use tempfile::TempDir;

/// Real prose from this repository's README.
const BODY: &str = "comemory keeps a durable, searchable memory of the \
    decisions, bugs and conventions a codebase accumulates, and links them to \
    the code they describe.";

struct Home {
    _dir: TempDir,
    paths: Paths,
    cfg: Config,
    conn: Connection,
}

fn home() -> Home {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(dir.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    Home {
        _dir: dir,
        paths,
        cfg: Config::defaults(),
        conn,
    }
}

impl Home {
    fn ctx(&mut self) -> Ctx<'_> {
        Ctx::borrowed(&self.paths, &self.cfg, &mut self.conn)
    }

    fn save(&mut self, body: &str) -> String {
        let request = memories::save::Request {
            body: body.to_string(),
            title: None,
            kind: Kind::Decision,
            repo: "Falconiere/comemory".to_string(),
            tags: vec!["sync".to_string()],
            author: "tester".to_string(),
            quality: 4,
            supersedes: Vec::new(),
            vector: None,
            ref_file: Vec::new(),
            ref_symbol: Vec::new(),
        };
        let mut ctx = self.ctx();
        memories::save::run(&mut ctx, request, false, None)
            .expect("save")
            .id
    }

    fn reconcile(&mut self) -> Report {
        let paths = self.paths.clone();
        recover::reconcile(&paths, &mut self.conn).expect("reconcile")
    }

    fn live_ids(&self) -> Vec<String> {
        comemory::store::memory_row::live_ids(&self.conn).expect("live ids")
    }

    fn feed_ops(&self) -> Vec<String> {
        replica_read::page(&self.conn, 0, 50, None)
            .expect("page")
            .into_iter()
            .map(|row| format!("{}:{}", row.op.as_str(), row.entity_key))
            .collect()
    }

    fn intents(&self) -> Vec<Intent> {
        memory_intent::outstanding(&self.conn).expect("outstanding")
    }
}

/// The state a process killed between the markdown rename and the mirror
/// commit leaves: the intent recorded, the file on disk, the database blind.
fn interrupted_save(home: &mut Home, body: &str) -> (String, std::path::PathBuf) {
    let store = MemoryStore::new(home.paths.clone());
    let entity_key = id::memory_id(body);
    let planned = store.planned_path(body);
    memory_intent::record(
        &home.conn,
        &Intent {
            entity_key: entity_key.clone(),
            kind: IntentKind::Write,
            md_path: planned.to_string_lossy().into_owned(),
            operation_id: format!("op-20260922-{}", &entity_key[..8]),
            started_at: "2026-09-22T10:00:00Z".to_string(),
        },
    )
    .expect("record intent");
    store
        .save(SaveParams {
            body,
            kind: Kind::Decision,
            repo: "Falconiere/comemory",
            tags: &["sync".to_string()],
            author: "tester",
            quality: 4,
            relations: Relations::default(),
            references: References::default(),
            created: None,
        })
        .expect("markdown lands");
    (entity_key, planned)
}

#[test]
fn an_interrupted_save_is_mirrored_and_journalled_by_the_next_pass() {
    let mut home = home();
    let (id, planned) = interrupted_save(&mut home, BODY);
    assert!(planned.exists());
    assert!(home.live_ids().is_empty(), "the database has never seen it");

    let report = home.reconcile();

    assert_eq!(report, Report { finished: 1, dropped: 0 });
    assert_eq!(home.live_ids(), vec![id.clone()], "the memory is findable");
    assert_eq!(
        home.feed_ops(),
        vec![format!("upsert:{id}")],
        "and the operation it owed is journalled"
    );
    let pending = replica_outbox::pending(&home.conn, 10).expect("pending");
    assert_eq!(pending.len(), 1, "the upload is owed");
    assert_eq!(pending[0].entity_key, id);
    assert!(home.intents().is_empty(), "nothing left outstanding");
}

#[test]
fn a_second_pass_over_a_reconciled_directory_changes_nothing() {
    let mut home = home();
    interrupted_save(&mut home, BODY);
    let first = home.reconcile();
    let feed_after_first = home.feed_ops();

    let second = home.reconcile();

    assert_eq!(first, Report { finished: 1, dropped: 0 });
    assert_eq!(second, Report::default(), "idempotent");
    assert_eq!(
        home.feed_ops(),
        feed_after_first,
        "no second position for one write"
    );
}

#[test]
fn the_operation_the_intent_named_is_the_one_the_recovered_write_journals() {
    let mut home = home();
    let (id, _) = interrupted_save(&mut home, BODY);
    let expected = home.intents()[0].operation_id.clone();

    home.reconcile();

    let feed = replica_read::page(&home.conn, 0, 10, None).expect("page");
    assert_eq!(feed.len(), 1);
    assert_eq!(feed[0].entity_key, id);
    assert_eq!(
        feed[0].operation_id, expected,
        "the recovered write adopts the recorded id rather than minting a second"
    );
}

#[test]
fn an_intent_whose_markdown_never_landed_is_dropped_without_a_trace() {
    let mut home = home();
    memory_intent::record(
        &home.conn,
        &Intent {
            entity_key: "deadbeef".to_string(),
            kind: IntentKind::Write,
            md_path: home
                .paths
                .memories_dir()
                .join("deadbeef-a-write-that-never-landed.md")
                .to_string_lossy()
                .into_owned(),
            operation_id: "op-20260922-deadbeef".to_string(),
            started_at: "2026-09-22T10:00:00Z".to_string(),
        },
    )
    .expect("record intent");

    let report = home.reconcile();

    assert_eq!(report, Report { finished: 0, dropped: 1 });
    assert!(home.live_ids().is_empty(), "no memory");
    assert!(home.feed_ops().is_empty(), "no operation");
    assert!(home.intents().is_empty(), "no orphan row");
}

#[test]
fn a_mirrored_write_whose_journal_never_committed_is_journalled_once() {
    let mut home = home();
    let id = home.save(BODY);
    // The state `update` and `restore` can reach: mirror committed, journal
    // not. Built by clearing the feed the real save wrote and re-recording
    // the intent, which is exactly what the crash leaves behind.
    home.conn
        .execute_batch("DELETE FROM replica_feed; DELETE FROM replica_revision; DELETE FROM replica_operation;")
        .expect("undo the journal half");
    let store = MemoryStore::new(home.paths.clone());
    memory_intent::record(
        &home.conn,
        &Intent {
            entity_key: id.clone(),
            kind: IntentKind::Write,
            md_path: store.planned_path(BODY).to_string_lossy().into_owned(),
            operation_id: "op-20260922-unjourna".to_string(),
            started_at: "2026-09-22T10:00:00Z".to_string(),
        },
    )
    .expect("record intent");

    let report = home.reconcile();

    assert_eq!(report, Report { finished: 1, dropped: 0 });
    assert_eq!(home.live_ids(), vec![id.clone()], "the row is still there");
    assert_eq!(
        home.feed_ops(),
        vec![format!("upsert:{id}")],
        "and the operation it owed now exists — exactly once"
    );
}

#[test]
fn an_interrupted_delete_is_completed_and_journalled_by_the_next_pass() {
    let mut home = home();
    let id = home.save(BODY);
    let live_path = MemoryStore::new(home.paths.clone()).planned_path(BODY);
    // The state a process killed between the `.trash/` move and the delete
    // transaction leaves: the file is gone from the live tree, the mirror row
    // is still live, and the tombstone was never journalled.
    let trash = home.paths.trash_dir();
    std::fs::create_dir_all(&trash).expect("trash dir");
    std::fs::rename(
        &live_path,
        trash.join(live_path.file_name().expect("file name")),
    )
    .expect("move to trash");
    memory_intent::record(
        &home.conn,
        &Intent {
            entity_key: id.clone(),
            kind: IntentKind::Delete,
            md_path: live_path.to_string_lossy().into_owned(),
            operation_id: "op-20260922-halfdele".to_string(),
            started_at: "2026-09-22T10:00:00Z".to_string(),
        },
    )
    .expect("record intent");
    assert_eq!(home.live_ids(), vec![id.clone()], "still live in the mirror");

    let report = home.reconcile();

    assert_eq!(report, Report { finished: 1, dropped: 0 });
    assert!(home.live_ids().is_empty(), "the delete completed");
    assert_eq!(
        home.feed_ops(),
        vec![format!("upsert:{id}"), format!("tombstone:{id}")],
        "and the tombstone is journalled"
    );
    assert!(home.intents().is_empty());
}

#[test]
fn a_delete_whose_markdown_never_moved_is_dropped_and_the_memory_stays_live() {
    let mut home = home();
    let id = home.save(BODY);
    let live_path = MemoryStore::new(home.paths.clone()).planned_path(BODY);
    memory_intent::record(
        &home.conn,
        &Intent {
            entity_key: id.clone(),
            kind: IntentKind::Delete,
            md_path: live_path.to_string_lossy().into_owned(),
            operation_id: "op-20260922-nevermov".to_string(),
            started_at: "2026-09-22T10:00:00Z".to_string(),
        },
    )
    .expect("record intent");

    let report = home.reconcile();

    assert_eq!(report, Report { finished: 0, dropped: 1 });
    assert!(live_path.exists(), "the markdown is untouched");
    assert_eq!(home.live_ids(), vec![id.clone()], "the memory is still live");
    assert_eq!(
        home.feed_ops(),
        vec![format!("upsert:{id}")],
        "no tombstone for a deletion that never happened"
    );
    assert!(home.intents().is_empty());
}

#[test]
fn a_read_only_session_reconciles_nothing_and_leaves_the_intent_standing() {
    let mut home = home();
    interrupted_save(&mut home, BODY);
    let paths = home.paths.clone();

    let report = recover::reconcile_unless_read_only(&paths, &mut home.conn, true)
        .expect("read-only reconcile");

    assert_eq!(report, Report::default());
    assert_eq!(
        home.intents().len(),
        1,
        "the unfinished write keeps until a writable open"
    );
    assert!(home.live_ids().is_empty(), "and nothing was written");
}

#[test]
fn a_clean_directory_reconciles_to_an_empty_report() {
    let mut home = home();
    home.save(BODY);
    assert_eq!(home.reconcile(), Report::default());
}
