#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Accepting a code generation against a real migrated database — the one
//! instant a whole generation becomes visible, and the four ways an
//! incomplete one stays invisible.

use crate::domains::code::replica_payload::{
    CODE_ENTITY_KIND, CODE_PAYLOAD_VERSION, CodeGenerationV1,
};
use crate::domains::sync::replica::contract::{Disposition, Operation, PROTOCOL};
use crate::domains::sync::replica::contract_views::{ActivateRequest, StageRequest};
use crate::domains::sync::replica::{accept, changes, manifest, staging, validate};
use crate::errors::Error;
use crate::prelude::Result;
use crate::store::remote_code::{Edge, File, Projection, Symbol};
use crate::store::replica_journal::ReplicaOp;
use crate::store::{code_generation, remote_code, replica_read};

use crate::domains::sync::replica::test_support as support;

use support::{Home, envelope};

const REPO: &str = "Falconiere/comemory";

/// The manifest a generation published.
fn files(conn: &rusqlite::Connection, repo: &str, generation_id: &str) -> Result<Vec<File>> {
    Ok(remote_code::projection(conn, repo, generation_id)?.files)
}

/// The symbols a generation published.
fn symbols(conn: &rusqlite::Connection, repo: &str, generation_id: &str) -> Result<Vec<Symbol>> {
    Ok(remote_code::projection(conn, repo, generation_id)?.symbols)
}

/// The edges a generation published.
fn edges(conn: &rusqlite::Connection, repo: &str, generation_id: &str) -> Result<Vec<Edge>> {
    Ok(remote_code::projection(conn, repo, generation_id)?.edges)
}

/// A projection with two files, a symbol, an import and a co-change pair.
fn projection(weight: i64) -> Projection {
    Projection {
        files: vec![
            File {
                path: "src/lib.rs".to_string(),
                blob_oid: "aaaa1111".to_string(),
            },
            File {
                path: "src/store.rs".to_string(),
                blob_oid: "bbbb2222".to_string(),
            },
        ],
        symbols: vec![Symbol {
            path: "src/lib.rs".to_string(),
            symbol: "run".to_string(),
            kind: "function".to_string(),
            lang: "rust".to_string(),
            line_start: 10,
            line_end: 42,
        }],
        edges: vec![Edge {
            rel: "co_changed".to_string(),
            src_path: "src/lib.rs".to_string(),
            dst_path: "src/store.rs".to_string(),
            weight,
            anchor: Some("commit-7".to_string()),
        }],
    }
}

/// A minted generation payload.
fn payload(parent: Option<&str>, head: &str, weight: i64) -> CodeGenerationV1 {
    let base = CodeGenerationV1::new("", parent, head, Some("commit-7"), &projection(weight));
    let id = base.mint_id().expect("mint");
    base.with_id(&id)
}

/// An upsert operation carrying `payload`.
fn upsert(operation_id: &str, payload: &CodeGenerationV1) -> Operation {
    let (bytes, digest) = payload.canonical().expect("canonical");
    Operation {
        operation_id: operation_id.to_string(),
        entity_kind: CODE_ENTITY_KIND.to_string(),
        entity_key: REPO.to_string(),
        op: ReplicaOp::Upsert,
        schema_version: CODE_PAYLOAD_VERSION,
        payload_digest: Some(digest),
        payload: Some(serde_json::from_str(&bytes).expect("json")),
        observed_sequence: None,
        repository: Some(REPO.to_string()),
        vector: None,
    }
}

#[test]
fn an_accepted_generation_becomes_visible_whole() {
    let mut home = Home::new();
    let payload = payload(None, "head-1", 3);
    let mut ctx = home.ctx();

    let response = accept::run(
        &mut ctx,
        envelope(vec![upsert("op-20260922-gen00001", &payload)]),
    )
    .expect("accept");

    assert_eq!(response.results[0].disposition, Disposition::Accepted);
    let active = code_generation::active(&home.conn, REPO)
        .expect("active")
        .expect("the generation is the repo's current one");
    assert_eq!(active.generation_id, payload.generation_id);
    assert_eq!(active.head, "head-1");
    assert_eq!(
        files(&home.conn, REPO, &payload.generation_id)
            .expect("files")
            .len(),
        2,
        "the whole manifest landed with the activation"
    );
    assert_eq!(
        edges(&home.conn, REPO, &payload.generation_id)
            .expect("edges")
            .len(),
        1
    );
    assert_eq!(
        replica_read::head(&home.conn).expect("head"),
        1,
        "and it earned exactly one feed position"
    );
}

#[test]
fn a_replayed_generation_replays_its_receipt_and_inflates_no_weight() {
    let mut home = Home::new();
    let payload = payload(None, "head-1", 3);
    let operation = upsert("op-20260922-gen00002", &payload);
    let mut ctx = home.ctx();
    let first = accept::run(&mut ctx, envelope(vec![operation.clone()])).expect("first");
    assert_eq!(first.results[0].disposition, Disposition::Accepted);

    let mut ctx = home.ctx();
    let replay = accept::run(&mut ctx, envelope(vec![operation])).expect("replay");

    assert_eq!(
        replay.results[0].disposition,
        Disposition::Duplicate,
        "the retry reads back its receipt"
    );
    assert_eq!(replay.results[0].sequence, first.results[0].sequence);
    let edges = edges(&home.conn, REPO, &payload.generation_id).expect("edges");
    assert_eq!(edges.len(), 1);
    assert_eq!(
        edges[0].weight, 3,
        "a replay writes the same rows; weights are the generation's, not a total"
    );
    assert_eq!(replica_read::head(&home.conn).expect("head"), 1);
}

#[test]
fn a_second_generation_supersedes_the_first_and_replaces_its_projection() {
    let mut home = Home::new();
    let first = payload(None, "head-1", 3);
    {
        let mut ctx = home.ctx();
        accept::run(
            &mut ctx,
            envelope(vec![upsert("op-20260922-gen00003", &first)]),
        )
        .expect("first");
    }
    let second = payload(Some(&first.generation_id), "head-2", 9);

    let mut ctx = home.ctx();
    let response = accept::run(
        &mut ctx,
        envelope(vec![upsert("op-20260922-gen00004", &second)]),
    )
    .expect("second");

    assert_eq!(response.results[0].disposition, Disposition::Accepted);
    let active = code_generation::active(&home.conn, REPO)
        .expect("active")
        .expect("row");
    assert_eq!(active.generation_id, second.generation_id);
    assert_eq!(active.head, "head-2");
    let prior = code_generation::by_id(&home.conn, REPO, &first.generation_id)
        .expect("by_id")
        .expect("row");
    assert_eq!(prior.state, code_generation::State::Superseded);
}

#[test]
fn a_generation_planned_against_a_stale_parent_never_activates() {
    let mut home = Home::new();
    let first = payload(None, "head-1", 3);
    {
        let mut ctx = home.ctx();
        accept::run(
            &mut ctx,
            envelope(vec![upsert("op-20260922-gen00005", &first)]),
        )
        .expect("first");
    }
    let second = payload(Some(&first.generation_id), "head-2", 9);
    {
        let mut ctx = home.ctx();
        accept::run(
            &mut ctx,
            envelope(vec![upsert("op-20260922-gen00006", &second)]),
        )
        .expect("second");
    }
    // A third machine planned against the first generation, not the second —
    // the concurrent case. Accepting it would union two heads.
    let stale = payload(Some(&first.generation_id), "head-3", 1);

    let mut ctx = home.ctx();
    let refused = accept::run(
        &mut ctx,
        envelope(vec![upsert("op-20260922-gen00007", &stale)]),
    );

    // The activation refuses inside the transaction, so nothing is written.
    assert!(refused.is_err(), "a stale plan cannot activate");
    let active = code_generation::active(&home.conn, REPO)
        .expect("active")
        .expect("row");
    assert_eq!(
        active.generation_id, second.generation_id,
        "the repo did not move, and no third projection exists"
    );
    assert!(
        files(&home.conn, REPO, &stale.generation_id)
            .expect("files")
            .is_empty(),
        "the refused generation published nothing"
    );
}

#[test]
fn an_authoritative_empty_generation_clears_the_projection() {
    let mut home = Home::new();
    let first = payload(None, "head-1", 3);
    {
        let mut ctx = home.ctx();
        accept::run(
            &mut ctx,
            envelope(vec![upsert("op-20260922-gen00008", &first)]),
        )
        .expect("first");
    }
    let tombstone = Operation {
        operation_id: "op-20260922-gen00009".to_string(),
        entity_kind: CODE_ENTITY_KIND.to_string(),
        entity_key: REPO.to_string(),
        op: ReplicaOp::Tombstone,
        schema_version: CODE_PAYLOAD_VERSION,
        payload_digest: None,
        payload: None,
        observed_sequence: None,
        repository: Some(REPO.to_string()),
        vector: None,
    };

    let mut ctx = home.ctx();
    let response = accept::run(&mut ctx, envelope(vec![tombstone])).expect("tombstone");

    assert_eq!(response.results[0].disposition, Disposition::Accepted);
    assert!(
        files(&home.conn, REPO, &first.generation_id)
            .expect("files")
            .is_empty(),
        "an empty authoritative generation removes the projection"
    );
}

#[test]
fn a_generation_that_does_not_own_its_id_is_refused_before_anything_is_written() {
    let mut home = Home::new();
    let mut tampered = payload(None, "head-1", 3);
    tampered.head = "a-different-head".to_string();
    let operation = upsert("op-20260922-gen00010", &tampered);

    let mut ctx = home.ctx();
    let response = accept::run(&mut ctx, envelope(vec![operation])).expect("accept");

    assert_eq!(
        response.results[0].disposition,
        Disposition::RejectedInvalid
    );
    assert_eq!(
        code_generation::active(&home.conn, REPO).expect("active"),
        None,
        "nothing activated"
    );
    assert_eq!(replica_read::head(&home.conn).expect("head"), 0);
}

#[test]
fn a_memory_operation_still_takes_the_memory_path() {
    let mut home = Home::new();
    let id = home.save(support::BODY, &["sync"]);
    let memory = home.payload(&id);

    let mut ctx = home.ctx();
    let decided = validate::decide(&mut ctx, &support::upsert("op-20260922-mem00001", &memory))
        .expect("decide");

    assert_eq!(
        decided,
        Disposition::RejectedStale,
        "the local save is still pending, so the memory guard applies unchanged"
    );
}

// ---------------------------------------------------------------------------
// AC-2 / AC-9: a generation crosses the wire in bounded parts, and every way
// that upload can fail leaves the last complete generation standing.
// ---------------------------------------------------------------------------

/// A projection of `count` real-shaped files — past `MAX_BATCH_FILES`, the
/// bound the pusher cuts a repository's manifest on.
fn wide_projection(count: usize) -> Projection {
    let files: Vec<File> = (0..count)
        .map(|n| File {
            path: format!("src/module_{n:04}.rs"),
            blob_oid: format!("{n:040x}"),
        })
        .collect();
    let symbols = files
        .iter()
        .map(|f| Symbol {
            path: f.path.clone(),
            symbol: "run".to_string(),
            kind: "function".to_string(),
            lang: "rust".to_string(),
            line_start: 1,
            line_end: 9,
        })
        .collect();
    Projection {
        files,
        symbols,
        edges: Vec::new(),
    }
}

/// A minted generation over `count` files.
fn wide_payload(count: usize) -> CodeGenerationV1 {
    let base = CodeGenerationV1::new("", None, "head-wide", None, &wide_projection(count));
    let id = base.mint_id().expect("mint");
    base.with_id(&id)
}

/// Cut `bytes` into `parts` pieces on character boundaries.
fn cut(bytes: &str, parts: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(parts);
    let mut start = 0;
    for index in 0..parts {
        let mut end = if index + 1 == parts {
            bytes.len()
        } else {
            bytes.len() * (index + 1) / parts
        };
        while !bytes.is_char_boundary(end) {
            end += 1;
        }
        out.push(bytes[start..end].to_string());
        start = end;
    }
    out
}

/// Send one part through the real `/stage` route.
fn stage_part(home: &mut Home, staging_id: &str, index: i64, count: i64, bytes: &str) {
    let mut ctx = home.ctx();
    staging::stage(
        &mut ctx,
        StageRequest {
            protocol: PROTOCOL.to_string(),
            staging_id: staging_id.to_string(),
            part_index: index,
            part_count: count,
            bytes: bytes.to_string(),
        },
    )
    .expect("stage");
}

/// The activation of a staged upload, whose operation carries no payload.
fn activation(staging_id: &str, operation_id: &str, payload: &CodeGenerationV1) -> ActivateRequest {
    ActivateRequest {
        protocol: PROTOCOL.to_string(),
        staging_id: staging_id.to_string(),
        operation: Operation {
            payload: None,
            ..upsert(operation_id, payload)
        },
        cursor: None,
    }
}

#[test]
fn a_generation_past_the_batch_bound_arrives_in_parts_and_activates_as_one_unit() {
    const FILES: usize = crate::domains::sync::code_plan::MAX_BATCH_FILES + 37;
    let mut home = Home::new();
    let payload = wide_payload(FILES);
    let (bytes, _) = payload.canonical().expect("canonical");
    let parts = cut(&bytes, 3);
    for (index, part) in parts.iter().enumerate() {
        stage_part(
            &mut home,
            "upload-wide",
            i64::try_from(index).expect("index"),
            3,
            part,
        );
    }

    let mut ctx = home.ctx();
    let result = staging::activate(
        &mut ctx,
        activation("upload-wide", "op-20260922-genwide1", &payload),
    )
    .expect("activate");

    assert_eq!(result.disposition, Disposition::Accepted);
    assert_eq!(
        files(&home.conn, REPO, &payload.generation_id)
            .expect("files")
            .len(),
        FILES,
        "every file of a multi-part generation is visible at once, or none is"
    );
    assert_eq!(
        symbols(&home.conn, REPO, &payload.generation_id)
            .expect("symbols")
            .len(),
        FILES
    );
    assert_eq!(
        replica_read::head(&home.conn).expect("head"),
        1,
        "three parts earned one feed position, not three"
    );
}

#[test]
fn a_withheld_part_leaves_the_generation_invisible_to_changes_and_manifest() {
    let mut home = Home::new();
    let payload = payload(None, "head-1", 3);
    let (bytes, _) = payload.canonical().expect("canonical");
    let parts = cut(&bytes, 2);
    stage_part(&mut home, "upload-withheld", 0, 2, &parts[0]);

    let mut ctx = home.ctx();
    let refused = staging::activate(
        &mut ctx,
        activation("upload-withheld", "op-20260922-genpart1", &payload),
    )
    .expect_err("a missing part refuses activation");

    assert!(matches!(refused, Error::Conflict(_)), "got {refused:?}");
    let mut ctx = home.ctx();
    assert!(
        changes::run(&mut ctx, 0, 100, None, None)
            .expect("changes")
            .entries
            .is_empty(),
        "a half-uploaded generation is not history"
    );
    let mut ctx = home.ctx();
    assert!(
        manifest::run(&mut ctx)
            .expect("manifest")
            .entity_kinds
            .is_empty(),
        "and it is nothing the manifest counts"
    );
    assert_eq!(
        code_generation::active(&home.conn, REPO).expect("active"),
        None
    );
    assert!(
        crate::store::replica_staging::assemble(&home.conn, "upload-withheld")
            .expect("assemble")
            .is_none(),
        "and nothing assembles from it"
    );
    // The part that DID arrive is kept: an incomplete upload is a retry, not
    // a failure, so the sender sends only the part still missing.
    stage_part(&mut home, "upload-withheld", 1, 2, &parts[1]);
    let mut ctx = home.ctx();
    let completed = staging::activate(
        &mut ctx,
        activation("upload-withheld", "op-20260922-genpart1", &payload),
    )
    .expect("activate once whole");
    assert_eq!(completed.disposition, Disposition::Accepted);
}

#[test]
fn a_corrupted_part_is_refused_and_leaves_the_last_generation_active() {
    let mut home = Home::new();
    let good = payload(None, "head-1", 3);
    {
        let mut ctx = home.ctx();
        accept::run(
            &mut ctx,
            envelope(vec![upsert("op-20260922-gencorr1", &good)]),
        )
        .expect("first");
    }

    // The second generation's bytes are mangled in flight: still JSON, but no
    // longer the bytes whose digest the operation declares.
    let next = payload(Some(&good.generation_id), "head-2", 9);
    let (bytes, _) = next.canonical().expect("canonical");
    let mangled = bytes.replace("\"head-2\"", "\"head-9\"");
    assert_ne!(mangled, bytes, "the corruption must change the bytes");
    stage_part(&mut home, "upload-corrupt", 0, 1, &mangled);

    let mut ctx = home.ctx();
    let result = staging::activate(
        &mut ctx,
        activation("upload-corrupt", "op-20260922-gencorr2", &next),
    )
    .expect("activate answers");

    assert_eq!(result.disposition, Disposition::RejectedInvalid);
    let active = code_generation::active(&home.conn, REPO)
        .expect("active")
        .expect("row");
    assert_eq!(
        active.generation_id, good.generation_id,
        "the last complete generation is still what the repo is at"
    );
    assert!(
        files(&home.conn, REPO, &next.generation_id)
            .expect("files")
            .is_empty(),
        "the refused generation published nothing"
    );
    assert_eq!(
        crate::store::replica_staging::assemble(&home.conn, "upload-corrupt").expect("assemble"),
        None,
        "an answered upload keeps no parts: its operation id can never accept"
    );
}

#[test]
fn a_part_over_the_envelope_cap_is_refused_and_the_staged_set_survives_for_retry() {
    let mut home = Home::new();
    let payload = payload(None, "head-1", 3);
    let (bytes, _) = payload.canonical().expect("canonical");
    let parts = cut(&bytes, 2);
    stage_part(&mut home, "upload-quota", 0, 2, &parts[0]);

    // The server refuses the next part on its own cap — the engine's real
    // rejection, not a simulated one.
    let over_cap = "x".repeat(crate::domains::sync::replica::contract::MAX_ENVELOPE_BYTES + 1);
    let mut ctx = home.ctx();
    let refused = staging::stage(
        &mut ctx,
        StageRequest {
            protocol: PROTOCOL.to_string(),
            staging_id: "upload-quota".to_string(),
            part_index: 1,
            part_count: 2,
            bytes: over_cap,
        },
    )
    .expect_err("a part over the cap is refused");

    assert!(matches!(refused, Error::BadRequest(_)), "got {refused:?}");
    // The refusal cost only the oversized part: the sender re-cuts it and the
    // generation still activates.
    stage_part(&mut home, "upload-quota", 1, 2, &parts[1]);
    let mut ctx = home.ctx();
    let result = staging::activate(
        &mut ctx,
        activation("upload-quota", "op-20260922-genquota", &payload),
    )
    .expect("activate");

    assert_eq!(result.disposition, Disposition::Accepted);
    assert_eq!(
        code_generation::active(&home.conn, REPO)
            .expect("active")
            .expect("row")
            .generation_id,
        payload.generation_id
    );
}

// ---------------------------------------------------------------------------
// AC-4 / AC-5: a pulled generation is a second, parallel body of knowledge.
// The local index owns every row `index-code` wrote, and upload selection
// offers only what this machine built.
// ---------------------------------------------------------------------------

/// Everything the local index owns for a repo: the rows an import must leave
/// byte-identical, plus the hits `search-code` answers from them.
#[derive(Debug, PartialEq)]
struct LocalRows {
    /// `code_symbols`, including the snippet a pulled generation never
    /// carries.
    symbols: Vec<(i64, String, String, String, i64)>,
    /// `code_vec`, as stored bytes.
    vectors: Vec<(i64, Vec<u8>)>,
    /// `repo_marker`: where the repo is, what head it was indexed at, and how
    /// far its history was mined.
    marker: (Option<String>, Option<String>, Option<String>),
    /// `indexed_files`, the cursor the next incremental walk reads.
    files: Vec<(String, String)>,
    /// What `comemory search-code` answers over those rows.
    hits: Vec<(i64, String, String, i64, i64)>,
}

/// Read every local row for `repo`, through the driver rather than a store
/// helper: this asserts over the stored bytes, including the columns no
/// accessor exposes.
fn local_rows(home: &Home, repo: &str) -> LocalRows {
    let mut statement = home
        .conn
        .prepare(
            "SELECT id, path, symbol, snippet, simhash FROM code_symbols \
             WHERE repo = ?1 ORDER BY id",
        )
        .expect("prepare symbols");
    let symbols = statement
        .query_map([repo], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .expect("query symbols")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("symbols");
    drop(statement);

    let mut statement = home
        .conn
        .prepare(
            "SELECT symbol_id, embedding FROM code_vec WHERE symbol_id IN \
             (SELECT id FROM code_symbols WHERE repo = ?1) ORDER BY symbol_id",
        )
        .expect("prepare vectors");
    let vectors = statement
        .query_map([repo], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query vectors")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("vectors");
    drop(statement);

    let mut files: Vec<(String, String)> =
        crate::store::indexed_files::list_for_repo(&home.conn, repo)
            .expect("indexed_files")
            .into_iter()
            .collect();
    files.sort();

    LocalRows {
        symbols,
        vectors,
        marker: (
            crate::store::repo_marker::last_head(&home.conn, repo).expect("head"),
            crate::store::repo_marker::root_path(&home.conn, repo).expect("root"),
            crate::store::repo_marker::last_mined_commit(&home.conn, repo).expect("mined"),
        ),
        files,
        hits: crate::domains::retrieval::code_search::search_code_hits(
            &home.cfg,
            &home.conn,
            // A symbol the fixture checkout really defines.
            "helper",
            None,
            Some(repo),
            None,
            20,
        )
        .expect("search-code")
        .into_iter()
        .map(|h| (h.symbol_id, h.path, h.symbol, h.line_start, h.line_end))
        .collect(),
    }
}

/// Index a real git checkout under `REPO` and give every symbol a stored
/// vector, so the import has local rows of every kind to leave alone.
fn indexed_locally(home: &mut Home, repo_path: &std::path::Path) {
    {
        let mut ctx = home.ctx();
        crate::domains::code::index_code::run(
            &mut ctx,
            crate::domains::code::index_code::Request {
                repo: REPO.to_string(),
                path: repo_path.to_str().expect("utf8 path").to_string(),
                mode: crate::domains::code::index_code::IndexMode::Incremental,
            },
        )
        .expect("index_code run");
    }
    let ids: Vec<i64> = {
        let mut statement = home
            .conn
            .prepare("SELECT id FROM code_symbols WHERE repo = ?1 ORDER BY id")
            .expect("prepare ids");
        statement
            .query_map([REPO], |r| r.get(0))
            .expect("query ids")
            .collect::<rusqlite::Result<Vec<i64>>>()
            .expect("ids")
    };
    let dims = crate::store::vector::dim_code(&home.conn).expect("dim_code");
    for id in ids {
        // A deterministic vector per symbol, at the table's own width — real
        // rows through the real writer, so the import has `code_vec` content
        // to preserve.
        #[allow(clippy::cast_precision_loss)]
        let vector: Vec<f32> = (0..dims)
            .map(|n| (id as f32) + (n as f32) / (dims as f32))
            .collect();
        crate::store::vector::insert_code(&home.conn, id, &vector).expect("insert_code");
    }
}

#[test]
fn importing_a_peers_generation_leaves_every_local_row_untouched() {
    let workspace = tempfile::tempdir().expect("workspace");
    let repo_path = crate::test_common::git_sample::build_sample_repo(workspace.path());
    let mut home = Home::new();
    indexed_locally(&mut home, &repo_path);
    let before = local_rows(&home, REPO);
    assert!(
        !before.symbols.is_empty() && !before.vectors.is_empty() && !before.files.is_empty(),
        "the fixture must have local rows to protect"
    );
    assert!(
        !before.hits.is_empty(),
        "and search-code must actually answer from them, or the comparison \
         below proves nothing: {before:?}"
    );
    // A peer indexed the same repo label at a head this machine has never
    // seen, and its manifest names paths the local checkout does not have.
    let peer = payload(None, "a-head-this-machine-never-saw", 3);

    let mut ctx = home.ctx();
    let response = accept::run(
        &mut ctx,
        envelope(vec![upsert("op-20260922-genlocal1", &peer)]),
    )
    .expect("accept");

    assert_eq!(response.results[0].disposition, Disposition::Accepted);
    assert_eq!(
        local_rows(&home, REPO),
        before,
        "an import writes only the pulled projection: no local row moved"
    );
    assert_eq!(
        files(&home.conn, REPO, &peer.generation_id)
            .expect("files")
            .len(),
        2,
        "and the peer's manifest did land, in its own tables"
    );
}

#[test]
fn a_pulled_generation_is_not_offered_as_this_machines_next_parent() {
    let workspace = tempfile::tempdir().expect("workspace");
    let repo_path = crate::test_common::git_sample::build_sample_repo(workspace.path());
    let mut home = Home::new();
    indexed_locally(&mut home, &repo_path);
    // This machine's own generation is what the repo is at.
    let local = crate::domains::code::generation::plan(&home.conn, REPO)
        .expect("plan")
        .expect("an indexed repo has a generation");
    code_generation::record(&home.conn, &local.generation, "2026-09-22T10:00:00Z").expect("record");
    code_generation::activate(
        &home.conn,
        REPO,
        &local.generation.generation_id,
        "2026-09-22T10:01:00Z",
    )
    .expect("activate");

    // The peer pulled this machine's generation before building its own, so
    // its plan names it as the parent — the repo has one chain, whoever
    // extends it.
    let peer = payload(
        Some(&local.generation.generation_id),
        "a-head-this-machine-never-saw",
        3,
    );
    {
        let mut ctx = home.ctx();
        accept::run(
            &mut ctx,
            envelope(vec![upsert("op-20260922-genlocal2", &peer)]),
        )
        .expect("accept");
    }

    let next = crate::domains::code::generation::plan(&home.conn, REPO)
        .expect("plan")
        .expect("row");
    assert_eq!(
        next.generation.parent_id.as_deref(),
        None,
        "the peer's generation is active, and it is not this machine's base"
    );
    assert_eq!(
        code_generation::active_local(&home.conn, REPO).expect("active_local"),
        None,
        "upload selection offers nothing while a pulled generation is active"
    );
    assert_eq!(
        code_generation::by_id(&home.conn, REPO, &local.generation.generation_id)
            .expect("by_id")
            .expect("row")
            .origin,
        crate::store::replica_journal::ReplicaOrigin::Local,
        "and this machine's own generation is still recorded as its own"
    );
}

// ---------------------------------------------------------------------------
// AC-8: a deletion is not a message of its own. The next generation simply
// does not name the path, and activation replaces the whole projection — so
// the path disappears once, and a replay removes nothing further.
// ---------------------------------------------------------------------------

/// A real checkout with two tracked source files.
fn two_file_repo(root: &std::path::Path) -> std::path::PathBuf {
    let repo = crate::test_common::git_sample::build_sample_repo(root);
    crate::test_common::git_commit::commit_files(
        &repo,
        &[("extra.rs", "fn extra() {}\n")],
        "add extra",
    );
    repo
}

/// Index `root` under `REPO` into `home`.
fn index_into(
    home: &mut Home,
    root: &std::path::Path,
    mode: crate::domains::code::index_code::IndexMode,
) {
    let mut ctx = home.ctx();
    crate::domains::code::index_code::run(
        &mut ctx,
        crate::domains::code::index_code::Request {
            repo: REPO.to_string(),
            path: root.to_str().expect("utf8 path").to_string(),
            mode,
        },
    )
    .expect("index_code run");
}

/// Plan the next generation, record and activate it locally, and return the
/// wire payload a push would send — minted from the same projection, so its
/// id must be the one the planner already computed.
fn plan_and_publish(home: &mut Home, at: &str) -> CodeGenerationV1 {
    let planned = crate::domains::code::generation::plan(&home.conn, REPO)
        .expect("plan")
        .expect("an indexed repo has a generation");
    let base = CodeGenerationV1::new(
        "",
        planned.generation.parent_id.as_deref(),
        &planned.generation.head,
        planned.generation.mined_commit.as_deref(),
        &planned.projection,
    );
    let id = base.mint_id().expect("mint");
    assert_eq!(
        id, planned.generation.generation_id,
        "the wire payload and the planner derive one identity"
    );
    code_generation::record(&home.conn, &planned.generation, at).expect("record");
    code_generation::activate(&home.conn, REPO, &id, at).expect("activate");
    base.with_id(&id)
}

#[test]
fn a_tracked_deletion_leaves_the_path_out_of_the_next_generation() {
    let workspace = tempfile::tempdir().expect("workspace");
    let root = two_file_repo(workspace.path());
    let mut author = Home::new();
    index_into(
        &mut author,
        &root,
        crate::domains::code::index_code::IndexMode::Incremental,
    );
    let first = plan_and_publish(&mut author, "2026-09-22T10:00:00Z");
    assert!(
        first
            .projection()
            .files
            .iter()
            .any(|f| f.path == "extra.rs"),
        "the first generation names both files"
    );

    let mut peer = Home::new();
    {
        let mut ctx = peer.ctx();
        accept::run(
            &mut ctx,
            envelope(vec![upsert("op-20260922-gendel01", &first)]),
        )
        .expect("first");
    }

    // The file is really deleted from the checkout and the repo re-indexed.
    std::fs::remove_file(root.join("extra.rs")).expect("remove");
    crate::test_common::git_commit::commit_files(&root, &[], "drop extra");
    // A full re-walk is what forgets a deleted file's cursor locally: the
    // incremental walk only ever visits files that still exist, so it has no
    // occasion to notice one that does not.
    index_into(
        &mut author,
        &root,
        crate::domains::code::index_code::IndexMode::Full,
    );
    let second = plan_and_publish(&mut author, "2026-09-22T11:00:00Z");

    assert_eq!(
        second.parent_id.as_deref(),
        Some(first.generation_id.as_str()),
        "the next generation extends the one that was published"
    );
    assert!(
        !second
            .projection()
            .files
            .iter()
            .any(|f| f.path == "extra.rs"),
        "and simply does not name the deleted path: {:?}",
        second.projection().files
    );

    let operation = upsert("op-20260922-gendel02", &second);
    {
        let mut ctx = peer.ctx();
        accept::run(&mut ctx, envelope(vec![operation.clone()])).expect("second");
    }

    let after: Vec<String> = files(&peer.conn, REPO, &second.generation_id)
        .expect("files")
        .into_iter()
        .map(|f| f.path)
        .collect();
    assert!(
        !after.iter().any(|p| p == "extra.rs"),
        "the peer no longer holds the deleted path: {after:?}"
    );
    assert!(
        after.iter().any(|p| p == "src.rs"),
        "and still holds the one that survived: {after:?}"
    );

    // A replay removes nothing further: the whole projection is replaced, so
    // the second application writes exactly the same rows.
    let mut ctx = peer.ctx();
    let replay = accept::run(&mut ctx, envelope(vec![operation])).expect("replay");
    assert_eq!(replay.results[0].disposition, Disposition::Duplicate);
    assert_eq!(
        files(&peer.conn, REPO, &second.generation_id)
            .expect("files")
            .into_iter()
            .map(|f| f.path)
            .collect::<Vec<_>>(),
        after
    );
}

#[test]
fn a_sweep_leaves_an_accepted_generations_replay_replaying() {
    let mut home = Home::new();
    let payload = payload(None, "head-1", 3);
    let operation = upsert("op-20260922-gensweep1", &payload);
    let first = {
        let mut ctx = home.ctx();
        accept::run(&mut ctx, envelope(vec![operation.clone()])).expect("accept")
    };
    // Debris from an upload that never finished, on the same database.
    crate::store::replica_staging::put_part(
        &home.conn,
        "upload-abandoned",
        0,
        2,
        "{}",
        "2026-09-20T12:00:00Z",
    )
    .expect("part");

    let swept = crate::store::replica_sweep::run(
        &home.conn,
        time::macros::datetime!(2026-09-22 12:00:00 UTC),
    )
    .expect("sweep");

    assert_eq!(swept.parts, 1);
    assert_eq!(swept.generations, 0, "the accepted generation is active");
    let mut ctx = home.ctx();
    let replay = accept::run(&mut ctx, envelope(vec![operation])).expect("replay");
    assert_eq!(
        replay.results[0].disposition,
        Disposition::Duplicate,
        "the receipt survived, so the retry is still answered from it"
    );
    assert_eq!(replay.results[0].sequence, first.results[0].sequence);
    assert_eq!(
        replica_read::head(&home.conn).expect("head"),
        1,
        "and no second position was earned"
    );
}
