#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`crate::sync::allowlist_cache`].

use std::time::Duration;

use comemory::config::paths::Paths;
use comemory::sync::{AllowlistCache, MatchOutcome};
use comemory::sync::{AllowlistRepo, classify_with_cache};
use time::OffsetDateTime;

use crate::test_common as common;

fn sample_cache() -> AllowlistCache {
    AllowlistCache {
        etag: Some("\"abc\"".into()),
        fetched_at: OffsetDateTime::now_utc(),
        repos: vec![AllowlistRepo {
            full_name: "codasignal/foo".into(),
            name: "foo".into(),
        }],
        workspace_id: "ws-org".into(),
    }
}

#[test]
fn allowlist_cache_roundtrip_and_classify() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().expect("ensure_dirs");
    let cache = sample_cache();
    cache.save(&paths).expect("save");

    let loaded = AllowlistCache::load(&paths)
        .expect("load")
        .expect("present");
    assert_eq!(loaded.workspace_id, "ws-org");
    assert_eq!(loaded.classify("foo"), MatchOutcome::Allowed);
    assert_eq!(
        loaded.classify("my-dotfiles"),
        MatchOutcome::SkippedNotInOrg
    );
}

#[test]
fn allowlist_cache_is_fresh_within_ttl() {
    let cache = sample_cache();
    assert!(cache.is_fresh(Duration::from_secs(3600)));
}

#[test]
fn classify_with_cache_returns_none_when_stale() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut cache = sample_cache();
    cache.fetched_at = OffsetDateTime::now_utc() - time::Duration::hours(2);
    cache.save(&paths).expect("save");

    let outcome = classify_with_cache(&paths, "foo", Duration::from_secs(60)).expect("classify");
    assert!(outcome.is_none());
}
