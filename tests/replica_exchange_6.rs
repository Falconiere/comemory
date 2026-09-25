#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The exchange client (#255), part 6: verification and rebootstrap across a
//! restored or replaced upstream (AC-13), a workspace switch that never
//! leaks a row across keys (AC-14), and a legacy import converging with a
//! replica push on the same workspace (AC-15).
//!
//! Real `comemory serve` hubs, the real CLI binary, real SQLite, real HTTP
//! through the fault proxy.

#[path = "common/exchange_support.rs"]
mod exchange_support;
#[path = "common/fault_proxy.rs"]
mod fault_proxy;
#[path = "common/replica_support.rs"]
mod replica_support;

use std::time::Duration;

use exchange_support::{Client, Hub, REPO, WORKSPACE, guide_body};
use fault_proxy::Fault;
use serde_json::{Value, json};

/// A client with `REPO` approved on `hub`.
fn approved_client(hub: &Hub) -> Client {
    let client = Client::new();
    client.login(hub);
    client.approve(&hub.api_url(), WORKSPACE, &[REPO], 1);
    client
}

/// One memory posted straight to an engine's own HTTP surface, bypassing any
/// client — what "console" or "another key" traffic looks like.
fn post_memory(
    http: &reqwest::blocking::Client,
    base: &str,
    token: &str,
    body: &str,
    repo: &str,
) -> String {
    let response = http
        .post(format!("{base}/api/v1/memories"))
        .bearer_auth(token)
        .json(&json!({"body": body, "kind": "decision", "repo": repo}))
        .send()
        .expect("post memory over http");
    let parsed: Value = response.json().expect("memory response is json");
    parsed["data"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("no id in response: {parsed}"))
        .to_string()
}

/// The `{kind, differing_buckets, repaired}` row for `kind` in a `verify`
/// report, or a panic naming what was actually reported.
fn kind_row<'a>(report: &'a Value, kind: &str) -> &'a Value {
    report["kinds"]
        .as_array()
        .unwrap_or_else(|| panic!("verify report has no kinds array: {report}"))
        .iter()
        .find(|k| k["kind"] == kind)
        .unwrap_or_else(|| panic!("{kind} missing from verify kinds: {report}"))
}

// ---------------------------------------------------------------------------
// AC-13 (X-7): verify reports zero differing buckets after a full drain of
// three kinds; a restore from an earlier copy and a replace both rebootstrap;
// pending operations survive and go out; verify reports what the upstream
// lost rather than a false match.
// ---------------------------------------------------------------------------

#[test]
fn verify_and_rebootstrap_after_stream_changes() {
    let mut hub = Hub::start();
    let a = approved_client(&hub);

    // A: three memories, a copy of this repo's docs/guides, and a small
    // pinned code checkout — one drain, three kinds.
    let m1 = a.save("verify: a memory A owns before anything is restored", REPO);
    let m2 = a.save(
        "verify: a second memory, also bound before the restore",
        REPO,
    );
    let m3 = a.save(
        "verify: a third memory, also bound before the restore",
        REPO,
    );

    // Documents are shared only from under the label's indexed root, so the
    // docs copy lives inside the pinned code checkout indexed first.
    let workspace = tempfile::tempdir().expect("workspace");
    let code_root = replica_support::pinned_repo(workspace.path(), 30);
    replica_support::git(
        &code_root,
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
        code_root.to_str().expect("utf8 code root"),
    ]);
    let docs_root = replica_support::docs_tree(workspace.path(), "pinned-repo");
    let guides_dir = docs_root.join("docs/guides");
    a.cli(&[
        "index",
        guides_dir.to_str().expect("utf8 guides dir"),
        "--repo",
        REPO,
    ]);

    a.sync();

    let b = approved_client(&hub);
    b.sync();
    let pulled = b.memory_ids();
    assert!(
        pulled.contains(&m2) && pulled.contains(&m3),
        "B pulled every memory A pushed: {pulled:?}"
    );

    let initial = b.cli(&["sync", "--action", "verify"]);
    for kind in ["memory", "code_generation", "document_revision"] {
        let row = kind_row(&initial, kind);
        assert_eq!(row["differing_buckets"], 0, "{kind} starts clean: {row}");
    }

    // 1,500 memories written straight to the hub, so a kind-scoped replay of
    // the sparse kinds below has to cross long runs of empty pages.
    let http = reqwest::blocking::Client::new();
    let hub_base = hub.engine().base.clone();
    let hub_token = hub.token();
    for n in 0..1500 {
        post_memory(&http, &hub_base, &hub_token, &guide_body(5000 + n), REPO);
    }

    // A snapshot of the hub holding the 1,500 memories but not the documents
    // pushed next — the point every later restore returns to (AC-13: "restored
    // from an earlier copy (1,500 memories, docs pushed after the copy)").
    let snapshot = tempfile::tempdir().expect("snapshot dir");
    hub.snapshot_to(snapshot.path());

    // A adds three more document revisions after the snapshot — the
    // difference the restore below is going to make the hub forget.
    for n in 0..3 {
        std::fs::write(
            guides_dir.join(format!("verify-extra-{n}.md")),
            format!("# Extra guide {n}\n\n{}\n", guide_body(6000 + n)),
        )
        .expect("write extra doc");
    }
    a.cli(&[
        "index",
        guides_dir.to_str().expect("utf8 guides dir"),
        "--repo",
        REPO,
    ]);
    a.sync();

    b.sync(); // pulls the 1,500 memories and the 3 later docs, before the restore

    // A pending local save on B, never synced before the restore — it has to
    // survive the rebootstrap and still go out. (Not pushed inline: an
    // acknowledged operation the restore loses is #256's to re-offer.)
    std::fs::write(
        b.data_dir().join("config.toml"),
        "[sync]\npush_on_save = false\n",
    )
    .expect("config");
    let bp = b.save(
        "verify: pending on B when the hub gets restored under it",
        REPO,
    );

    hub.restore_from(snapshot.path()); // same epoch, head back below B's cursor
    b.login(&hub); // the restore restarted the engine, rotating its token
    a.login(&hub);

    // A patch to a memory both keys already bound, made straight on the
    // restored hub — what a console edit or another key's write looks like.
    // It lands at a position the truncated-and-regrown feed reuses.
    let (patch_status, _) = hub.engine().patch(
        &format!("/api/v1/memories/{m1}"),
        &json!({"tags": ["post-restore-patch"]}),
    );
    assert!(
        patch_status < 300,
        "the post-restore patch is accepted: {patch_status}"
    );

    // Two SIGKILLs during the rebootstrap over the 1,500-entry hub: neither
    // one is allowed to leave B looking caught up. Each kill lands while the
    // rebootstrap's pull is parked on the proxy — after the pass began it —
    // so it is never a race against how fast the machine drains.
    for attempt in 0..2 {
        hub.proxy.arm(Fault::HoldRequest {
            path: "/sync/replica/changes".into(),
        });
        let mut child = b.spawn(&["sync"], &[]);
        assert!(
            hub.proxy.wait_held(Duration::from_mins(1)),
            "attempt {attempt}: the rebootstrap's pull is parked"
        );
        child
            .kill()
            .unwrap_or_else(|e| panic!("kill attempt {attempt}: {e}"));
        let _ = child.wait();
        hub.proxy.clear();
        let status = b.exchange_status();
        assert_ne!(
            status["caught_up"], true,
            "not caught up mid-rebootstrap after a kill (attempt {attempt}): {status}"
        );
    }

    // The run that actually finishes the rebootstrap.
    let result = b.sync();
    assert_eq!(
        result["exchange"]["rebootstrapped"], true,
        "restoring from an earlier copy is a rebootstrap: {result}"
    );

    let status = b.exchange_status();
    assert_eq!(
        status["caught_up"], true,
        "caught up once the run finished: {status}"
    );

    // bp survived the rebootstrap and was pushed; B has the post-restore patch.
    let hub_feed = hub.feed();
    assert!(
        hub_feed.iter().any(|(_, key, _)| key == &bp),
        "the pending local save reached the restored hub: {hub_feed:?}"
    );

    // B's own memory as it stands now (the feed's first entry for m1 is its
    // creation, not the patch).
    let b_engine = b.serve();
    let (status, shown) = b_engine.get(&format!("/api/v1/memories/{m1}"));
    drop(b_engine);
    assert_eq!(status, 200, "{shown}");
    let m1_tags: Vec<String> =
        serde_json::from_value(shown["data"]["tags"].clone()).unwrap_or_default();
    assert!(
        m1_tags.iter().any(|t| t == "post-restore-patch"),
        "the post-restore patch reached B: {m1_tags:?}"
    );

    // Verify reports the real difference: the upstream lost the 3 later docs
    // and never gets them back; memory repairs clean.
    let verify = b.cli(&["sync", "--action", "verify"]);
    let memory_row = kind_row(&verify, "memory");
    assert_eq!(
        memory_row["differing_buckets"], 0,
        "memory repairs clean: {memory_row}"
    );

    let doc_row = kind_row(&verify, "document_revision");
    assert!(
        doc_row["differing_buckets"].as_i64().unwrap_or(0) > 0,
        "document_revision still differs — the upstream lost the 3 later docs: {doc_row}"
    );
    assert_eq!(
        doc_row["repaired"], false,
        "the loss is reported, not silently repaired: {doc_row}"
    );

    let doc_kind_requests = hub.proxy.requests_to("kind=document_revision");
    assert!(
        doc_kind_requests.len() >= 2,
        "the sparse document_revision kind is scanned across several pages: {}",
        doc_kind_requests.len()
    );

    // A brand-new epoch: also a rebootstrap, and B's pending work survives it.
    let after_replace = b.save(
        "verify: kept across a brand-new stream epoch and pushed once it opens",
        REPO,
    );
    hub.replace();
    b.login(&hub);
    let replaced = b.sync();
    assert_eq!(
        replaced["exchange"]["rebootstrapped"], true,
        "a brand-new epoch is also a rebootstrap: {replaced}"
    );
    let hub_feed_after_replace = hub.feed();
    assert!(
        hub_feed_after_replace
            .iter()
            .any(|(_, key, _)| key == &after_replace),
        "the pending save survived the replace and was pushed: {hub_feed_after_replace:?}"
    );
}

/// A restored stream that carries a peer's edit followed by this client's own
/// accepted edit ends with the client's own text locally, matching its
/// binding — the client's own operation wins over an interleaved peer edit,
/// not the other way around.
#[test]
fn rebootstrap_keeps_the_clients_own_accepted_edit_over_a_peer_edit() {
    let mut h2 = Hub::start();
    let a = approved_client(&h2);
    let b = approved_client(&h2);

    let m = a.save("peer-then-own: the memory both keys bind to", REPO);
    a.sync();
    b.sync(); // B binds M

    let b_engine = b.serve();
    let (status, _) = b_engine.patch(&format!("/api/v1/memories/{m}"), &json!({"tags": ["op-b"]}));
    assert!(status < 300, "B's patch is accepted locally: {status}");
    drop(b_engine);
    b.sync(); // op_b reaches H2

    a.sync(); // A pulls op_b and binds it

    let a_engine = a.serve();
    let (status, _) = a_engine.patch(&format!("/api/v1/memories/{m}"), &json!({"tags": ["op-a"]}));
    assert!(status < 300, "A's patch is accepted locally: {status}");
    drop(a_engine);
    a.sync(); // op_a reaches H2, after op_b

    let snapshot = tempfile::tempdir().expect("snapshot dir");
    h2.snapshot_to(snapshot.path()); // both op_b and op_a are in this copy

    // Grow the live head past the snapshot with filler A then pulls, so a
    // restore from the snapshot lands below A's already-advanced cursor.
    let http = reqwest::blocking::Client::new();
    let filler_base = h2.engine().base.clone();
    let filler_token = h2.token();
    for n in 0..5 {
        post_memory(
            &http,
            &filler_base,
            &filler_token,
            &guide_body(7000 + n),
            REPO,
        );
    }
    a.sync(); // A's cursor now sits above the snapshot's head

    h2.restore_from(snapshot.path());
    a.login(&h2);
    b.login(&h2);

    let result = a.sync();
    assert_eq!(
        result["exchange"]["rebootstrapped"], true,
        "A's cursor was above the restored head: {result}"
    );

    let a_engine = a.serve();
    let (status, shown) = a_engine.get(&format!("/api/v1/memories/{m}"));
    let local_digest = replica_support::digest_of(&a_engine, &m);
    drop(a_engine);
    assert_eq!(status, 200, "{shown}");
    let tags: Vec<String> =
        serde_json::from_value(shown["data"]["tags"].clone()).unwrap_or_default();
    assert!(
        tags.iter().any(|t| t == "op-a"),
        "A ends with its own accepted edit's tags, not the peer's: {tags:?}"
    );

    let conn = a.open();
    let bound: Option<String> = conn
        .query_row(
            "SELECT synced_digest FROM replica_binding WHERE entity_kind = 'memory' AND entity_key = ?1",
            rusqlite::params![m],
            |r| r.get(0),
        )
        .expect("A has a binding for M");
    assert_eq!(
        bound.as_deref(),
        Some(local_digest.as_str()),
        "A's binding matches its own local revision for {m}"
    );
}

// ---------------------------------------------------------------------------
// AC-14 (X-8): a real logout and a credential for another workspace never
// cross rows between the two; the switch back does.
// ---------------------------------------------------------------------------

#[test]
fn workspace_switch_never_crosses() {
    let ha = Hub::start();
    let hb = Hub::start();

    let a = Client::new();
    a.write_auth(&ha.api_url(), &ha.token(), "ws_a");
    a.approve(&ha.api_url(), "ws_a", &[REPO], 1);
    // x1 and x2 must still be owed at the switch, not delivered inline.
    std::fs::write(
        a.data_dir().join("config.toml"),
        "[sync]\npush_on_save = false\n",
    )
    .expect("config");

    // A teammate's memory on HA, pulled by A before any of the switching below.
    let http = reqwest::blocking::Client::new();
    let p = post_memory(
        &http,
        &ha.engine().base,
        &ha.token(),
        "workspace-switch: a teammate's memory on workspace A",
        REPO,
    );
    a.sync(); // A pulls P

    let x1 = a.save(
        "workspace-switch: x1, made under A, never synced before the switch",
        REPO,
    );
    let x2 = a.save(
        "workspace-switch: x2, made under A, never synced before the switch",
        REPO,
    );

    // A real, offline logout: no remote revoke, just the local credential gone.
    let (code, _out, err) = a.cli_raw(&["auth", "logout"], &[]);
    assert!(
        code == 0 || !a.data_dir().join("auth.json").exists(),
        "logout should work offline, or at least remove the credential: exit {code} {err}"
    );
    assert!(
        !a.data_dir().join("auth.json").exists(),
        "auth.json is gone after logout"
    );

    // A credential for workspace B, written the way a login would.
    a.write_auth(&hb.api_url(), &hb.token(), "ws_b");
    a.approve(&hb.api_url(), "ws_b", &[REPO], 1);

    let status_before_first_hb_sync = a.exchange_status();
    assert_ne!(
        status_before_first_hb_sync["caught_up"], true,
        "B's key never inherits A's caught_up before its own first sync: {status_before_first_hb_sync}"
    );

    // P, pulled under A, patched now that the credential names B.
    let a_engine = a.serve();
    let (status, _) = a_engine.patch(
        &format!("/api/v1/memories/{p}"),
        &json!({"tags": ["patched-under-b"]}),
    );
    assert!(status < 300, "the patch is accepted locally: {status}");
    drop(a_engine);

    a.sync();

    let hb_feed = hb.feed();
    for entity in [x1.as_str(), x2.as_str(), p.as_str()] {
        assert!(
            !hb_feed.iter().any(|(_, key, _)| key == entity),
            "workspace B never receives {entity}: {hb_feed:?}"
        );
    }
    let hb_import_bodies: String = hb
        .proxy
        .requests_to("import")
        .into_iter()
        .map(|l| l.body)
        .collect::<Vec<_>>()
        .join("\n---\n");
    for entity in [x1.as_str(), x2.as_str(), p.as_str()] {
        assert!(
            !hb_import_bodies.contains(entity),
            "B's proxy log never even carried {entity}: {hb_import_bodies}"
        );
    }

    let status = a.exchange_status();
    assert!(
        status["outbox"]["held"]["workspace"].as_i64().unwrap_or(0) >= 3,
        "x1, x2 and the P patch are all held workspace: {status}"
    );
    assert_eq!(
        status["applied_sequence"],
        hb.head(),
        "B's cursor starts and stays exactly at B's own head, nothing inherited: {status}"
    );
    assert_eq!(status["workspace"], "ws_b", "{status}");

    // Switch back: A's credential, and its held rows finally go out.
    a.write_auth(&ha.api_url(), &ha.token(), "ws_a");
    a.sync();

    let feed_back_on_a = ha.feed();
    for entity in [x1.as_str(), x2.as_str(), p.as_str()] {
        assert!(
            feed_back_on_a.iter().any(|(_, key, _)| key == entity),
            "workspace A finally receives {entity}: {feed_back_on_a:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// AC-15 (X-8): a legacy import and a replica push converge on the engine's
// single history.
// ---------------------------------------------------------------------------

#[test]
fn legacy_and_replica_clients_converge() {
    let hub = Hub::start();

    // Build a valid legacy wire record from a scratch save, then post it to
    // the hub exactly as the platform forwards one.
    let scratch = Client::new();
    let scratch_engine = scratch.serve();
    let (status, saved) = scratch_engine.post(
        "/api/v1/memories",
        &json!({"body": "legacy-converge: a write the platform forwards untouched", "kind": "decision", "repo": REPO}),
    );
    assert!(status < 300, "scratch save: {status} {saved}");
    let legacy_id = saved["data"]["id"].as_str().expect("id").to_string();

    let (status, changes) = scratch_engine.get("/api/v1/sync/changes?since=0&limit=10");
    assert_eq!(status, 200, "{changes}");
    let mut entry = changes["data"]["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|e| e["id"] == legacy_id)
        .unwrap_or_else(|| panic!("no legacy entry for {legacy_id}: {changes}"))
        .clone();
    drop(scratch_engine);
    let object = entry.as_object_mut().expect("entry is an object");
    object.remove("seq");
    object.remove("author");

    let (status, imported) = hub.engine().post(
        "/api/v1/sync/import",
        &json!({"cursor": 0, "entries": [entry]}),
    );
    assert_eq!(status, 200, "legacy import to the hub: {imported}");

    let a = approved_client(&hub);
    a.sync();

    assert!(
        a.memory_ids().contains(&legacy_id),
        "A pulled the legacy write: {:?}",
        a.memory_ids()
    );
    let outbox_rows_for_legacy: i64 = {
        let conn = a.open();
        conn.query_row(
            "SELECT COUNT(*) FROM replica_operation WHERE entity_key = ?1",
            rusqlite::params![legacy_id],
            |r| r.get(0),
        )
        .expect("count outbox rows")
    };
    assert_eq!(
        outbox_rows_for_legacy, 0,
        "A never owes an upload for a write it only pulled"
    );

    let feed_len_before_second_sync = hub.feed().len();
    a.sync();
    assert_eq!(
        hub.feed().len(),
        feed_len_before_second_sync,
        "the pulled legacy entry is never pushed back"
    );

    let m2 = a.save(
        "legacy-converge: A's own write, pushed once through the replica protocol",
        REPO,
    );
    a.sync();

    let (status, legacy_changes) = hub.engine().get("/api/v1/sync/changes?since=0&limit=500");
    assert_eq!(status, 200, "{legacy_changes}");
    let occurrences = legacy_changes["data"]["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .filter(|e| e["id"] == m2)
        .count();
    assert_eq!(
        occurrences, 1,
        "M2 appears exactly once in the engine's legacy changes: {legacy_changes}"
    );
}
