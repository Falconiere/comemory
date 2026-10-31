#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Minimal requests for the exhaustive HTTP read-only gate sweep.

use comemory::serve::routes::RouteEntry;

/// A minimal, syntactically-complete JSON body (and, for routes whose
/// extractor needs a required query param, extra `(key, value)` pairs) for
/// one `mutating` route — just enough to satisfy the route's `Json`/`Query`
/// extractor so the request reaches the handler body and its read-only
/// gate, without asserting anything about the operation's actual outcome
/// (AC-4 only cares whether the gate fires). Every `mutating` command must
/// be listed here — an unmapped command panics rather than being silently
/// skipped, so a newly added mutating route is forced to extend this table.
pub fn minimal_request(
    entry: &RouteEntry,
) -> (serde_json::Value, Vec<(&'static str, &'static str)>) {
    match entry.command {
        "save" => (
            serde_json::json!({"body": "AC-4 sweep test memory"}),
            vec![],
        ),
        "feedback" => (serde_json::json!({"query_id": "q-sweep-deadbeef"}), vec![]),
        // An empty body is enough for every command below. `rebuild`, `tune`
        // and `bandit` carry no `confirm`/`apply`, so on a normal server they
        // stop at the confirm gate (`400`) and the sweep never actually
        // rebuilds or retunes — exactly what AC-4 wants, since it only
        // asserts whether the read-only gate fired.
        // `gc.run` stops at its own confirm gate and `doctor.reembed` at the
        // `503 embedder_unavailable` check (this sweep's server has no embed
        // command), so neither actually sweeps or re-embeds; `gc.policy.update`
        // with no fields patches nothing. All three still reach their
        // read-only gate, which is all AC-4 asserts.
        // The console-api routes (2026-09-01 spec) follow the same rule: an
        // empty body reaches each handler's read-only gate, and on a normal
        // server every one then stops at its own confirm gate, a `404` for
        // the dummy id, a `400` for the missing field, or the `501` a
        // deliberately unmodelled action answers — never a `405`.
        "delete"
        | "prune"
        | "gc"
        | "mine"
        | "install-hooks"
        | "rebuild"
        | "ingest-code"
        | "tune"
        | "bandit"
        | "gc.run"
        | "gc.policy.update"
        | "doctor.reembed"
        | "memories.update"
        | "memories.restore"
        | "memories.refresh-refs"
        | "trash.restore"
        | "repos.patch"
        | "repos.archive"
        | "repos.disconnect"
        | "graph.recompute"
        | "learning.proposals.apply"
        | "learning.proposals.discard"
        | "config.retrieval.update"
        | "memory-stores.patch"
        | "memory-stores.sync"
        // `erase` stops at its confirm gate on a normal server: an empty body
        // names no entity, and nothing is erased.
        | "erase"
        // `project.create` answers `400 name is required` to an empty body,
        // so no project is chartered; each lifecycle verb answers `400` for
        // its missing `idempotencyKey`, so no project moves, and neither does
        // `project.evidence.add`, so no evidence is recorded.
        // `project.proposal.submit` does the same for `idempotencyKey`, so
        // no proposal is written.
        | "project.create"
        | "project.archive"
        | "project.restore"
        | "project.pause"
        | "project.resume"
        | "project.evidence.add"
        | "project.proposal.submit" => (serde_json::json!({}), vec![]),
        // `PUT /hooks/{name}` needs a body and, like `hooks`, a repo path the
        // gate is checked before touching.
        "hooks.set" => (
            serde_json::json!({"enabled": true}),
            vec![("repo", "/nonexistent/ac4-sweep")],
        ),
        // A connect contains its root BEFORE anything is written, so a
        // nonexistent one is a `400` on a normal server.
        "repos.connect" => (
            serde_json::json!({"root": "/nonexistent/ac4-sweep"}),
            vec![],
        ),
        "search.feedback" => (
            serde_json::json!({"hit_id": "deadbeef", "signal": "used"}),
            vec![],
        ),
        "memory-stores.create" => (
            serde_json::json!({"path": "/nonexistent/ac4-sweep"}),
            vec![],
        ),
        // `hooks` needs a repo to act on. A nonexistent path is right for the
        // sweep: on a read-only server the gate must fire BEFORE any git-hook
        // file is touched, and on a normal server the request gets past the
        // gate (which is all AC-4 asserts) and then fails on the missing repo.
        "hooks" => (
            serde_json::json!({"repo": "/nonexistent/ac4-sweep", "enable": "post-commit"}),
            vec![],
        ),
        "unindex" => (
            serde_json::json!({}),
            vec![("target", "ac4-sweep-nonexistent")],
        ),
        // `POST /code/index` and `POST /index/runs` share one gate and one
        // job spawner, so they share one sweep body too.
        "index-code" | "index.run" | "run" => (
            serde_json::json!({"repo": "sweep", "path": "/nonexistent/ac4-sweep"}),
            vec![],
        ),
        "index" => (
            serde_json::json!({"path": ["/nonexistent/ac4-sweep"]}),
            vec![],
        ),
        // Empty batch is enough: the read-only gate fires before import work.
        "sync.import" => (serde_json::json!({"cursor": 0, "entries": []}), vec![]),
        // Same for the code half: a repo label and no files is a valid
        // (empty) batch, so a normal server accepts it past the gate.
        "sync.code.import" => (
            serde_json::json!({"repo": "ac4-sweep", "files": []}),
            vec![],
        ),
        // The replica half: an empty operation list is a valid envelope, so a
        // normal server accepts it past the gate and a read-only one refuses
        // before any journal write.
        "sync.replica.import" => (
            serde_json::json!({"protocol": "replica-v1", "operations": []}),
            vec![],
        ),
        // One tiny part of a one-part upload: enough to reach the gate, and
        // harmless on a normal server (staged parts publish nothing).
        "sync.replica.stage" => (
            serde_json::json!({
                "protocol": "replica-v1",
                "staging_id": "ac4-sweep",
                "part_index": 0,
                "part_count": 1,
                "bytes": "{}"
            }),
            vec![],
        ),
        // Activation of an upload that was never staged: past the gate on a
        // normal server, refused as incomplete rather than publishing.
        "sync.replica.activate" => (
            serde_json::json!({
                "protocol": "replica-v1",
                "staging_id": "ac4-sweep-missing",
                "operation": {
                    "operation_id": "op-20260921-ac4sweep",
                    "entity_kind": "memory",
                    "entity_key": "ac4a5eep",
                    "op": "tombstone",
                    "schema_version": 1
                }
            }),
            vec![],
        ),
        other => panic!(
            "minimal_request: no minimal body/query wired for mutating command {other:?} — \
             add one so the AC-4 read-only sweep stays exhaustive"
        ),
    }
}
