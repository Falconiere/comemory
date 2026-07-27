//! Test mirror for `src/retrieval/scope.rs`.

use comemory::retrieval::scope::TimeScope;

#[test]
fn none_is_the_unbounded_scope() {
    let scope = TimeScope::none();
    assert_eq!(scope.since, None);
    assert_eq!(scope.cutoff, None);
    assert!(!scope.as_of);
    assert!(scope.is_unbounded(), "no bound set must read as unbounded");
    assert_eq!(scope, TimeScope::default(), "none() is the Default value");
}

#[test]
fn any_bound_makes_the_scope_bounded() {
    let since_only = TimeScope {
        since: Some("2026-03-10T00:00:00Z".into()),
        ..TimeScope::none()
    };
    assert!(!since_only.is_unbounded());

    let cutoff_only = TimeScope {
        cutoff: Some("2026-03-10T23:59:59.999999999Z".into()),
        ..TimeScope::none()
    };
    assert!(!cutoff_only.is_unbounded());

    let both = TimeScope {
        since: Some("2026-03-01T00:00:00Z".into()),
        cutoff: Some("2026-03-10T23:59:59.999999999Z".into()),
        as_of: true,
    };
    assert!(!both.is_unbounded());
}

#[test]
fn as_of_alone_does_not_bound_the_window() {
    // `as_of` only refines what `cutoff` means, so it cannot make an
    // otherwise-empty scope bounded — the CLI never builds this shape,
    // but the predicate must not key on the flag.
    let flag_only = TimeScope {
        as_of: true,
        ..TimeScope::none()
    };
    assert!(flag_only.is_unbounded());
}
