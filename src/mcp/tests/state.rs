#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/mcp/state.rs` against a real database opened in a
//! tempdir: the session's accessors, and the two inputs `track()` answers to.
//!
//! `track()`'s env half is asserted as DELEGATION to
//! `config::env::access_tracking_enabled` rather than by mutating
//! `COMEMORY_DISABLE_ACCESS_TRACKING` in-process: flipping a process-global
//! env var needs `unsafe` in Rust 2024, and the repository proves that
//! variable's effect the way every other suite does — on a spawned binary
//! (`tests/cli__search.rs`, `tests/common/time_travel.rs`). What is mcp's
//! own rule, and is asserted here outright, is that `--read-only` answers
//! `false` before the env is ever consulted.

use comemory::config::{Config, Paths, env};
use comemory::domains::projects::authority::{Envelope, Verb};
use comemory::domains::projects::principal::{LOCAL_AGENT_ID, Principal, PrincipalType};
use comemory::mcp::McpOptions;
use comemory::mcp::state::McpState;
use comemory::store::memory_list::{ListFilter, SortBy, list_memories};
use comemory::utilities::error_code::classify;
use tempfile::{TempDir, tempdir};

/// A session over a real `comemory.db` under `dir`, scoped to `app`.
fn state_in(dir: &TempDir, read_only: bool) -> McpState {
    let paths = Paths::new(dir.path().to_path_buf());
    McpState::new(
        &paths,
        McpOptions {
            repo: Some("app".into()),
            read_only,
            cfg: Config::defaults(),
        },
        dir.path(),
    )
    .expect("open mcp state")
}

#[test]
fn new_opens_a_real_store_and_exposes_the_session() {
    let dir = tempdir().expect("tempdir");
    let state = state_in(&dir, false);

    assert!(
        dir.path().join("comemory.db").is_file(),
        "new() must open (and therefore create) the database"
    );
    assert!(
        dir.path().join("memories").is_dir(),
        "new() must ensure the data-dir tree"
    );
    assert_eq!(state.repo(), Some("app"));
    assert!(!state.read_only());
    assert_eq!(state.paths().db_path(), dir.path().join("comemory.db"));
    assert_eq!(
        state.cfg().retrieval.top_k,
        Config::defaults().retrieval.top_k
    );

    // A real query proves the connection is usable, not merely constructed.
    let guard = state.conn().expect("open a call connection");
    let page = list_memories(&guard, &ListFilter::default(), 10, 0, SortBy::Created)
        .expect("list on a fresh store");
    assert_eq!(page.total, 0);
    drop(guard);

    // A clone opens the same live database, so it sees the same store.
    let clone = state.clone();
    assert_eq!(clone.repo(), Some("app"));
    assert!(clone.conn().is_ok());
}

#[test]
fn a_session_started_outside_a_repo_and_without_a_flag_has_no_scope() {
    let dir = tempdir().expect("tempdir");
    let paths = Paths::new(dir.path().to_path_buf());
    let state = McpState::new(
        &paths,
        McpOptions {
            repo: None,
            read_only: true,
            cfg: Config::defaults(),
        },
        dir.path(),
    )
    .expect("open mcp state");
    assert_eq!(state.repo(), None, "a tempdir is inside no git work tree");
    assert!(state.read_only());
}

#[test]
fn track_is_false_for_a_read_only_session() {
    let dir = tempdir().expect("tempdir");
    let state = state_in(&dir, true);
    assert!(
        !state.track().expect("track on a read-only session"),
        "a --read-only session writes nothing, telemetry included"
    );
}

#[test]
fn track_delegates_to_the_shared_access_tracking_setting() {
    let dir = tempdir().expect("tempdir");
    let state = state_in(&dir, false);
    let shared = env::access_tracking_enabled().expect("read the shared setting");
    assert_eq!(
        state.track().expect("track on a writable session"),
        shared,
        "a writable session must follow config::env::access_tracking_enabled, \
         the one definition cli and serve also read"
    );
}

/// Every project tool runs as the local agent (#315): holding all six
/// capabilities, it is admitted on every shared verb, yet by principal kind
/// it cannot approve, request changes, reject, complete, cancel or delete —
/// on a read-write session as on a read-only one.
#[test]
fn the_project_envelope_holds_every_capability_and_no_human_verb() {
    use Verb as V;
    let dir = tempdir().expect("tempdir");
    let state = state_in(&dir, false);
    let envelope = state.project_envelope();
    assert_eq!(envelope, &Envelope::local_agent());
    assert_eq!(
        envelope.principal(),
        &Principal::new(PrincipalType::ProjectAgent, LOCAL_AGENT_ID)
    );
    for verb in [
        V::ProjectShow,
        V::ProjectList,
        V::PlanRead,
        V::ProposalRead,
        V::WorkItemRead,
        V::EvidenceRead,
        V::ActivityRead,
        V::ProposalCreate,
        V::WorkPacketCreate,
        V::WorkItemTransition,
        V::ExecutionUpdate,
        V::EvidenceCreate,
        V::HealthUpdate,
    ] {
        assert!(envelope.authorize(verb).is_ok(), "{verb:?} refused");
    }
    for verb in [
        V::ProposalApprove,
        V::ProposalRequestChanges,
        V::ProposalReject,
        V::ProjectComplete,
        V::WorkItemComplete,
        V::ProjectCancel,
        V::ProjectDelete,
    ] {
        let e = envelope.authorize(verb).expect_err("a human verb admitted");
        assert_eq!(classify(&e).0, "project_agent_scope", "{verb:?}");
        assert_eq!(e.to_string(), "This command requires a signed-in human");
    }
    let read_only = tempdir().expect("tempdir");
    assert_eq!(state_in(&read_only, true).project_envelope(), envelope);
}
