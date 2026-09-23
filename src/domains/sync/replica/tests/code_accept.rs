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
