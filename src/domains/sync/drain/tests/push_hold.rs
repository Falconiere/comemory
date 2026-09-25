#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Hold classification over real saves: each cause holds its own row, a row
//! behind a held one for the same entity waits in order, other entities flow,
//! and a hold clears when its cause is gone.

use std::collections::{BTreeMap, BTreeSet};

use crate::domains::sync::drain::push_hold::{self, Context};
use crate::domains::sync::replica::test_support::{BODY, Home};
use crate::domains::sync::repository_policy::RepositoryPolicy;
use crate::domains::sync::skip_repos::SkipMatcher;
use crate::store::replica_outbox::{self, Scope};
use crate::store::replica_outbox_hold::{self, Change, Target};
use crate::store::sync_exchange::ExchangeKey;
use crate::store::sync_policy_snapshot::{self, PolicySnapshot};

fn key() -> ExchangeKey {
    ExchangeKey::new("http://127.0.0.1:9/api", "ws")
}

fn policy(home: &Home, repos: &[&str]) -> RepositoryPolicy {
    sync_policy_snapshot::save(
        &home.conn,
        &key(),
        &PolicySnapshot {
            revision: 1,
            fingerprint: "fp".into(),
            allowlist: repos.iter().map(|r| (*r).to_string()).collect(),
            mappings: BTreeMap::new(),
            loaded_at: "t".into(),
        },
    )
    .expect("snapshot");
    RepositoryPolicy::from_snapshot(&home.conn, &key()).expect("policy")
}

fn capabilities(kinds: &[&str]) -> BTreeSet<String> {
    kinds.iter().map(|k| (*k).to_string()).collect()
}

fn holds(home: &Home) -> Vec<Option<String>> {
    replica_outbox::read(&home.conn, Scope::All, 50)
        .expect("read")
        .into_iter()
        .map(|r| r.hold_reason)
        .collect()
}

#[test]
fn an_approved_memory_is_eligible_and_an_unapproved_one_is_held_policy() {
    let mut home = Home::new();
    home.save(BODY, &["sync"]);
    let key = key();
    let skip = SkipMatcher::compile(&[]).expect("skip");
    let caps = capabilities(&["memory@1"]);

    let unapproved = policy(&home, &["acme/other"]);
    let ctx = Context {
        key: &key,
        policy: &unapproved,
        skip: &skip,
        capabilities: &caps,
        upgrading_since: None,
    };
    assert_eq!(
        push_hold::classify(&home.conn, &ctx, "t").expect("classify"),
        1
    );
    assert_eq!(holds(&home), vec![Some("policy".to_string())]);

    let approved = policy(&home, &["falconiere/comemory"]);
    let ctx = Context {
        key: &key,
        policy: &approved,
        skip: &skip,
        capabilities: &caps,
        upgrading_since: None,
    };
    assert_eq!(
        push_hold::classify(&home.conn, &ctx, "t").expect("again"),
        0
    );
    assert_eq!(
        holds(&home),
        vec![None],
        "the hold clears when the approval arrives"
    );
}

#[test]
fn skip_repos_incompatible_kinds_and_the_upgrade_horizon_each_hold() {
    let mut home = Home::new();
    home.save(BODY, &["sync"]);
    let key = key();
    let approved = policy(&home, &["falconiere/comemory"]);
    let none = SkipMatcher::compile(&[]).expect("skip");
    let skipped = SkipMatcher::compile(&["falconiere/*".to_string()]).expect("skip");
    let memory = capabilities(&["memory@1"]);
    let nothing = capabilities(&[]);

    let ctx = Context {
        key: &key,
        policy: &approved,
        skip: &skipped,
        capabilities: &memory,
        upgrading_since: None,
    };
    push_hold::classify(&home.conn, &ctx, "t").expect("skip");
    assert_eq!(holds(&home), vec![Some("skip_repos".to_string())]);

    let ctx = Context {
        key: &key,
        policy: &approved,
        skip: &none,
        capabilities: &nothing,
        upgrading_since: None,
    };
    push_hold::classify(&home.conn, &ctx, "t").expect("incompatible");
    assert_eq!(holds(&home), vec![Some("incompatible".to_string())]);

    let ctx = Context {
        key: &key,
        policy: &approved,
        skip: &none,
        capabilities: &memory,
        upgrading_since: Some("2999-01-01T00:00:00Z"),
    };
    push_hold::classify(&home.conn, &ctx, "t").expect("upgrade");
    assert_eq!(holds(&home), vec![Some("upgrade".to_string())]);
}

#[test]
fn a_secret_holds_its_memory_while_other_memories_flow() {
    let mut home = Home::new();
    home.save(
        "api_key=Zx9Qp2Lm7Rt4Wn8Yc3Vb6Hj1Ks5Fd0Ae was pasted into this note",
        &["sync"],
    );
    home.save(BODY, &["sync"]);
    let key = key();
    let approved = policy(&home, &["falconiere/comemory"]);
    let skip = SkipMatcher::compile(&[]).expect("skip");
    let caps = capabilities(&["memory@1"]);
    let ctx = Context {
        key: &key,
        policy: &approved,
        skip: &skip,
        capabilities: &caps,
        upgrading_since: None,
    };

    assert_eq!(
        push_hold::classify(&home.conn, &ctx, "t").expect("classify"),
        1
    );
    assert_eq!(holds(&home), vec![Some("secret".to_string()), None]);
}

#[test]
fn a_row_stamped_for_another_key_is_held_workspace_and_later_rows_wait_in_order() {
    let mut home = Home::new();
    let id = home.save(BODY, &["sync"]);
    let first = replica_outbox::read(&home.conn, Scope::All, 5)
        .expect("read")
        .remove(0);
    let other = ExchangeKey::new("http://127.0.0.1:9/api", "ws_other");
    replica_outbox_hold::update(
        &home.conn,
        Target::Operation(&first.operation_id),
        Change::Stamp(&other),
        "t",
    )
    .expect("stamp");
    // A later tag edit of the same memory.
    home.conn
        .execute(
            "UPDATE replica_operation SET created_at = '2000-01-01T00:00:00Z'",
            [],
        )
        .expect("age the first row");
    let mut ctx_home = home.ctx();
    crate::domains::memories::update::run(
        &mut ctx_home,
        &id,
        crate::domains::memories::update::Request {
            tags: Some(vec!["later".into()]),
            ..Default::default()
        },
    )
    .expect("patch");
    drop(ctx_home);
    let key = key();
    let approved = policy(&home, &["falconiere/comemory"]);
    let skip = SkipMatcher::compile(&[]).expect("skip");
    let caps = capabilities(&["memory@1"]);
    let ctx = Context {
        key: &key,
        policy: &approved,
        skip: &skip,
        capabilities: &caps,
        upgrading_since: None,
    };

    push_hold::classify(&home.conn, &ctx, "t").expect("classify");
    assert_eq!(
        holds(&home),
        vec![Some("workspace".to_string()), Some("order".to_string())],
        "the later edit of the same memory waits behind it"
    );
}
