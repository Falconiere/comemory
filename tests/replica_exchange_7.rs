#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The exchange client (#255), part 7: code and document generations big
//! enough to cross `stage`/`activate`, a delete-then-restore round trip
//! between two peers (AC-16); and the engine surfaces the exchange leans on
//! — a hub's own HTTP writes interleaved with a client's, seeding without
//! enqueueing an upload the hub owes to nobody, and a `comemory rebuild`
//! that must not forget a key's stamps, holds, bindings, cursor or anchor
//! (AC-17).
//!
//! Real `comemory serve` hubs, the real CLI binary, real SQLite, real HTTP.

#[path = "common/exchange_support.rs"]
mod exchange_support;
#[path = "common/fault_proxy.rs"]
mod fault_proxy;
#[path = "common/replica_support.rs"]
mod replica_support;

use std::path::Path;

use exchange_support::{Client, Hub, REPO, WORKSPACE, guide_body, prejournal_memories};
use rusqlite::types::Value as SqlValue;
use serde_json::json;

/// A client logged into `hub` with `REPO` approved — the same shape every
/// other exchange suite uses.
fn approved_client(hub: &Hub) -> Client {
    let client = Client::new();
    client.login(hub);
    client.approve(&hub.api_url(), WORKSPACE, &[REPO], 1);
    client
}

/// Every `.rs` file under `dir`, counted recursively — the size a pinned
/// checkout of this repository's own `src/` needs to cover it whole.
fn count_rs_files(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            count += count_rs_files(&path);
        } else if path.extension().is_some_and(|e| e == "rs") {
            count += 1;
        }
    }
    count
}

/// `(state, disposition, hold_reason)` of the most recently touched
/// `replica_operation` row for one entity, direct off the client's own
/// database — `Client::outbox` has no room for `disposition`.
fn latest_op_row(client: &Client, entity_key: &str) -> (String, Option<String>, Option<String>) {
    let conn = client.open();
    conn.query_row(
        "SELECT state, disposition, hold_reason FROM replica_operation \
         WHERE entity_key = ?1 ORDER BY updated_at DESC, rowid DESC LIMIT 1",
        [entity_key],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .unwrap_or_else(|e| panic!("no replica_operation row for {entity_key}: {e}"))
}

const CURSOR_SQL: &str = "SELECT api_url, workspace_id, stream_epoch, applied_sequence, \
     anchor_sequence, anchor_operation_id FROM replica_cursor ORDER BY api_url, workspace_id";
const BINDING_SQL: &str = "SELECT api_url, workspace_id, entity_kind, entity_key, synced_digest, \
     synced_deleted, synced_sequence, synced_epoch FROM replica_binding \
     ORDER BY entity_kind, entity_key";
const HOLD_SQL: &str = "SELECT api_url, workspace_id, stream_epoch, from_sequence, to_sequence, \
     reason, entity_kind, entity_key, repository, policy_revision FROM replica_pull_hold \
     ORDER BY api_url, workspace_id, stream_epoch, from_sequence";
const OPERATION_SQL: &str = "SELECT operation_id, entity_key, state, hold_reason, api_url, \
     workspace_id FROM replica_operation ORDER BY created_at, operation_id";

/// Every row `sql` selects off `client`'s own database, generically — good
/// enough to compare a table's whole content before and after a rebuild.
fn dump(client: &Client, sql: &str) -> Vec<Vec<SqlValue>> {
    let conn = client.open();
    let mut statement = conn.prepare(sql).expect("prepare");
    let columns = statement.column_count();
    let mut rows = statement.query([]).expect("query");
    let mut out = Vec::new();
    while let Some(row) = rows.next().expect("row") {
        out.push(
            (0..columns)
                .map(|i| row.get::<_, SqlValue>(i).expect("value"))
                .collect(),
        );
    }
    out
}

// ---------------------------------------------------------------------------
// AC-16: code and document generations, and a delete + restore round trip.
// ---------------------------------------------------------------------------

#[test]
fn code_documents_and_restore_drain_both_ways() {
    // A real operator behind a proxy with a small body limit lowers the
    // per-request budget; this repository's whole-src generation is then too
    // large for one request and must cross through stage/activate.
    const MAX_REQUEST_BYTES: usize = 256 * 1024;

    let hub = Hub::start();
    let a = approved_client(&hub);
    let workdir = tempfile::tempdir().expect("workdir");

    // Code: a pinned checkout of this repository's WHOLE `src/`, so the
    // generation it plans is big enough to force `stage`/`activate`.
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let file_count = count_rs_files(&src);
    assert!(
        file_count > 0,
        "this repository has Rust sources under src/"
    );
    let repo_path = replica_support::pinned_repo(workdir.path(), file_count);
    replica_support::git(
        &repo_path,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/Falconiere/comemory.git",
        ],
    );
    a.cli(&[
        "index-code",
        "--repo",
        REPO,
        "--path",
        repo_path.to_str().expect("utf8 path"),
    ]);

    let planned = replica_support::planned_generation(&a.data_dir(), REPO)
        .unwrap_or_else(|| panic!("A planned a code generation for {REPO}"));
    let serialized = serde_json::to_vec(&planned).expect("serialize planned generation");
    assert!(
        serialized.len() > MAX_REQUEST_BYTES,
        "a whole-src checkout's generation is {} bytes, expected more than the {} byte \
         request budget so this case actually forces stage/activate",
        serialized.len(),
        MAX_REQUEST_BYTES
    );
    std::fs::write(
        a.data_dir().join("config.toml"),
        format!("[sync]\nmax_request_bytes = {MAX_REQUEST_BYTES}\n"),
    )
    .expect("write config");

    // Documents: a copy of this repository's own docs/guides.
    // Inside the checkout `index-code` registered as REPO's root: documents
    // are shared only from under the label's indexed root.
    let docs_root = replica_support::docs_tree(workdir.path(), "pinned-repo");
    let guides_dir = docs_root.join("docs/guides");
    a.cli(&[
        "index",
        guides_dir.to_str().expect("utf8 path"),
        "--repo",
        REPO,
    ]);

    a.sync();
    assert!(
        !hub.proxy.requests_to("/sync/replica/stage").is_empty(),
        "the oversized generation crosses through stage: {:?}",
        hub.proxy.log()
    );
    assert!(
        !hub.proxy.requests_to("/sync/replica/activate").is_empty(),
        "and is activated once staged: {:?}",
        hub.proxy.log()
    );

    let code_symbols_before = {
        let conn = a.open();
        conn.query_row("SELECT COUNT(*) FROM code_symbols", [], |r| {
            r.get::<_, i64>(0)
        })
        .expect("count code_symbols")
    };

    let a_last_head = a.cli(&["repos"])["repos"]
        .as_array()
        .expect("repos array")
        .iter()
        .find(|r| r["repo"] == json!(REPO))
        .and_then(|r| r["last_head"].as_str())
        .map_or_else(
            || panic!("A's own repos inventory names {REPO}'s last_head"),
            str::to_string,
        );

    let b = approved_client(&hub);
    b.sync();

    let b_repos = b.cli(&["repos"]);
    let b_row = b_repos["repos"]
        .as_array()
        .expect("repos array")
        .iter()
        .find(|r| r["repo"] == json!(REPO))
        .unwrap_or_else(|| panic!("B lists the shared repository: {b_repos}"));
    assert_eq!(b_row["status"], "shared", "{b_row}");
    assert_eq!(
        b_row["shared_head"],
        json!(a_last_head),
        "B's shared generation is A's own head: {b_row}"
    );

    let phrase = "Share organization memories across machines";
    let hits = b.cli(&["search", "--only", "document", phrase]);
    let items = hits["items"].as_array().expect("items array");
    assert!(
        !items.is_empty(),
        "B finds a shared passage for {phrase:?}: {hits}"
    );
    assert!(
        items.iter().any(|h| !h["shared_from"].is_null()),
        "at least one hit names its shared origin: {hits}"
    );

    // A's own local rows are untouched by anything B did.
    a.sync();
    let code_symbols_after = {
        let conn = a.open();
        conn.query_row("SELECT COUNT(*) FROM code_symbols", [], |r| {
            r.get::<_, i64>(0)
        })
        .expect("count code_symbols")
    };
    assert_eq!(
        code_symbols_before, code_symbols_after,
        "B pulling and A syncing again never touches A's own code_symbols rows"
    );

    // Delete + restore, both ways.
    let r = a.save(&guide_body(7000), REPO);
    a.sync();
    b.sync();
    assert!(
        b.memory_ids().contains(&r),
        "B has R before it is deleted: {:?}",
        b.memory_ids()
    );

    a.cli(&["delete", &r]);
    a.sync();
    b.sync();
    assert!(
        !b.memory_ids().contains(&r),
        "B no longer has R live after the delete drains: {:?}",
        b.memory_ids()
    );

    let serve = a.serve();
    let (status, body) = serve.post(&format!("/api/v1/trash/{r}/restore"), &json!({}));
    assert!(
        status < 300,
        "A restores R through its own serve: {status} {body}"
    );
    drop(serve);

    a.sync();
    let (_, entity_key, op) = hub
        .feed()
        .into_iter()
        .rev()
        .find(|(_, key, _)| key == &r)
        .unwrap_or_else(|| panic!("the hub feed carries a position for R"));
    assert_eq!(
        op, "restore",
        "the hub feed's last op for R: entity {entity_key}"
    );

    let (state, disposition, _hold_reason) = latest_op_row(&a, &r);
    assert_eq!(
        state, "accepted",
        "A's restore is accepted, not stuck pending: {disposition:?}"
    );
    assert_ne!(
        disposition.as_deref(),
        Some("rejected_stale"),
        "and the upstream did not treat it as stale"
    );

    b.sync();
    assert!(
        b.memory_ids().contains(&r),
        "B has R live again after the restore drains: {:?}",
        b.memory_ids()
    );
}

// ---------------------------------------------------------------------------
// AC-17: the engine surfaces the exchange needs.
// ---------------------------------------------------------------------------

#[test]
fn engine_serves_the_exchange_it_needs() {
    // (a) hub HTTP writes interleaved with a client's own patch, accepted in
    // server order.
    let hub = Hub::start();
    let (status, saved) = hub.engine().post(
        "/api/v1/memories",
        &json!({"body": guide_body(7001), "kind": "decision", "repo": REPO}),
    );
    assert!(status < 300, "hub save: {status} {saved}");
    let h = saved["data"]["id"].as_str().expect("id").to_string();
    let (status, patched) = hub
        .engine()
        .patch(&format!("/api/v1/memories/{h}"), &json!({"tags": ["hub"]}));
    assert!(status < 300, "hub patch: {status} {patched}");

    let a = approved_client(&hub);
    a.sync();
    assert!(
        a.memory_ids().contains(&h),
        "A pulled H: {:?}",
        a.memory_ids()
    );

    let serve = a.serve();
    let (status, patched) = serve.patch(
        &format!("/api/v1/memories/{h}"),
        &json!({"tags": ["client"]}),
    );
    assert!(status < 300, "client patch: {status} {patched}");
    drop(serve);

    a.sync();
    let (state, disposition, _hold_reason) = latest_op_row(&a, &h);
    assert_eq!(
        state, "accepted",
        "A's patch of H is accepted in server order: {disposition:?}"
    );
    assert_ne!(disposition.as_deref(), Some("rejected_stale"));

    let (status, shown) = hub.engine().get(&format!("/api/v1/memories/{h}"));
    assert_eq!(status, 200, "{shown}");
    assert_eq!(
        shown["data"]["tags"],
        json!(["client"]),
        "the hub kept the client's patch: {shown}"
    );

    // (b) seeding journals without enqueueing an upload the hub owes nobody.
    let seeded = Hub::start_prepared(|data| prejournal_memories(data, 250, REPO));
    let _ = seeded.engine().get("/api/v1/sync/replica/manifest");
    let _ = seeded.engine().get("/api/v1/sync/replica/manifest");
    let conn = seeded.db();
    let operation_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM replica_operation", [], |r| r.get(0))
        .expect("count replica_operation");
    let feed_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM replica_feed", [], |r| r.get(0))
        .expect("count replica_feed");
    assert_eq!(
        operation_count, 0,
        "seeding never enqueues an upload for the hub itself"
    );
    assert!(
        feed_count >= 250,
        "seeding journals every pre-journal memory: {feed_count}"
    );

    // (c) a client rebuild keeps a key's exchange state — stamps, holds,
    // bindings, cursor and anchor — intact, and a stamped row still goes
    // only to its own key.
    let hub_x = Hub::start();
    let client = Client::new();
    client.login(&hub_x);
    client.approve(&hub_x.api_url(), WORKSPACE, &[REPO], 1);
    client.save(&guide_body(7002), "acme/private");
    client.save(&guide_body(7003), REPO);
    client.sync();
    client.sync(); // a second pass so the client's own accepted op is pulled back and anchors the cursor

    let status_before = client.exchange_status();
    assert!(
        status_before["outbox"]["held"]["policy"]
            .as_u64()
            .unwrap_or(0)
            >= 1,
        "the unapproved save stays held policy: {status_before}"
    );
    let cursor_before = dump(&client, CURSOR_SQL);
    let binding_before = dump(&client, BINDING_SQL);
    let hold_before = dump(&client, HOLD_SQL);
    let operation_before = dump(&client, OPERATION_SQL);

    client.cli(&["rebuild"]);

    let status_after = client.exchange_status();
    assert_eq!(
        status_before, status_after,
        "rebuild leaves the exchange status untouched"
    );
    assert_eq!(
        cursor_before,
        dump(&client, CURSOR_SQL),
        "rebuild keeps the cursor and its anchor"
    );
    assert_eq!(
        binding_before,
        dump(&client, BINDING_SQL),
        "rebuild keeps bindings"
    );
    assert_eq!(
        hold_before,
        dump(&client, HOLD_SQL),
        "rebuild keeps pull holds"
    );
    assert_eq!(
        operation_before,
        dump(&client, OPERATION_SQL),
        "rebuild keeps stamps"
    );

    let stamped_op_id = client
        .outbox()
        .into_iter()
        .find(|(_, state, hold_reason)| state == "accepted" && hold_reason.is_none())
        .map_or_else(
            || panic!("one save was accepted by hub_x: {:?}", client.outbox()),
            |(id, _, _)| id,
        );

    let hub_b = Hub::start();
    client.write_auth(&hub_b.api_url(), &hub_b.token(), WORKSPACE);
    client.approve(&hub_b.api_url(), WORKSPACE, &[REPO], 1);
    client.sync();

    assert!(
        !hub_b
            .feed()
            .into_iter()
            .any(|(op, _, _)| op == stamped_op_id),
        "the row stamped for hub_x's key never reaches hub_b"
    );
    let status_on_b = client.exchange_status();
    assert!(
        status_on_b["outbox"]["held"]["workspace"]
            .as_u64()
            .unwrap_or(0)
            >= 1,
        "and is counted held.workspace under the new key: {status_on_b}"
    );
}
