#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/mcp/exec.rs` against a real store: an `Access::Read`
//! drives a real command core (`domains::memories::list::run`) over a memory
//! written by a real `domains::memories::save::run`, and an `Access::Write`
//! proves a `--read-only` session never reaches its closure — asserted with a
//! flag the closure itself would set, not by reading the error alone.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use comemory::config::{Config, Paths};
use comemory::domains::memories::{Kind, list, save};
use comemory::errors::Error;
use comemory::mcp::McpOptions;
use comemory::mcp::exec::{self, Access};
use comemory::mcp::state::McpState;
use tempfile::{TempDir, tempdir};

const REPO: &str = "app";

fn state_in(dir: &TempDir, read_only: bool) -> McpState {
    let paths = Paths::new(dir.path().to_path_buf());
    McpState::new(
        &paths,
        McpOptions {
            repo: Some(REPO.into()),
            read_only,
            cfg: Config::defaults(),
        },
        dir.path(),
    )
    .expect("open mcp state")
}

/// The `save` request shape with only body and repo set.
fn save_request(body: &str) -> save::Request {
    save::Request {
        body: body.to_string(),
        title: None,
        kind: Kind::Note,
        repo: REPO.to_string(),
        tags: Vec::new(),
        author: String::new(),
        quality: 3,
        supersedes: Vec::new(),
        vector: None,
        ref_file: Vec::new(),
        ref_symbol: Vec::new(),
    }
}

#[tokio::test]
async fn a_read_drives_a_real_core_over_the_session_connection() {
    let dir = tempdir().expect("tempdir");
    let state = state_in(&dir, false);

    // Seed through the production writer, on the same shared connection.
    let saved = exec::run(state.clone(), Access::Write("save"), |ctx, _| {
        save::run(
            ctx,
            save_request("Prefer nextest over cargo test\n"),
            false,
            None,
        )
    })
    .await
    .expect("save through a write access");

    let page = exec::run(state.clone(), Access::Read, |ctx, st| {
        list::run(
            ctx,
            list::Request {
                repo: st.repo().map(str::to_string),
                kind: None,
                tag: None,
                min_quality: None,
                q: None,
                limit: 10,
                offset: 0,
                sort: list::Sort::Created,
            },
        )
    })
    .await
    .expect("list through a read access");

    assert_eq!(page.items.len(), 1, "the saved memory must come back");
    assert_eq!(page.items[0].id, saved.id);
}

#[tokio::test]
async fn a_write_on_a_read_only_session_never_reaches_the_closure() {
    let dir = tempdir().expect("tempdir");
    let state = state_in(&dir, true);
    let ran = Arc::new(AtomicBool::new(false));

    let flag = Arc::clone(&ran);
    let outcome = exec::run(state.clone(), Access::Write("save"), move |ctx, _| {
        flag.store(true, Ordering::SeqCst);
        save::run(ctx, save_request("must never be written\n"), false, None)
    })
    .await;

    match outcome {
        Err(Error::Forbidden(message)) => assert!(
            message.contains("read-only") && message.contains("save"),
            "refusal must name the tool and the reason: {message}"
        ),
        other => panic!("expected a read-only refusal, got {other:?}"),
    }
    assert!(
        !ran.load(Ordering::SeqCst),
        "the closure ran: a read-only session must refuse before any core is called"
    );

    // And nothing was written: a later read sees an empty store.
    let page = exec::run(state, Access::Read, |ctx, _| {
        list::run(
            ctx,
            list::Request {
                repo: None,
                kind: None,
                tag: None,
                min_quality: None,
                q: None,
                limit: 10,
                offset: 0,
                sort: list::Sort::Created,
            },
        )
    })
    .await
    .expect("list through a read access");
    assert!(page.items.is_empty(), "a refused write must leave no rows");
}

#[tokio::test]
async fn a_read_propagates_a_core_error_unchanged() {
    let dir = tempdir().expect("tempdir");
    let state = state_in(&dir, false);

    let outcome = exec::run(state, Access::Read, |ctx, _| {
        list::run(
            ctx,
            list::Request {
                repo: None,
                kind: None,
                tag: None,
                min_quality: Some(9),
                q: None,
                limit: 10,
                offset: 0,
                sort: list::Sort::Created,
            },
        )
    })
    .await;

    match outcome {
        Err(Error::BadRequest(message)) => {
            assert!(message.contains("min_quality"), "message: {message}");
        }
        Err(other) => panic!("expected the core's own BadRequest, got {other:?}"),
        Ok(page) => panic!("expected an error, got {} rows", page.items.len()),
    }
}
