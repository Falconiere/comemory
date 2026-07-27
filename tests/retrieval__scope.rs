//! Test mirror for `src/retrieval/scope.rs`.

use comemory::retrieval::scope::{Filters, TimeScope};

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

#[test]
fn window_borrows_both_bounds_and_drops_the_as_of_flag() {
    let scope = TimeScope {
        since: Some("2026-03-01T00:00:00Z".into()),
        cutoff: Some("2026-03-10T23:59:59.999999999Z".into()),
        as_of: true,
    };
    let window = scope.window();
    assert_eq!(window.since, Some("2026-03-01T00:00:00Z"));
    assert_eq!(window.cutoff, Some("2026-03-10T23:59:59.999999999Z"));

    let open = TimeScope::none();
    let unbounded = open.window();
    assert_eq!(unbounded.since, None);
    assert_eq!(unbounded.cutoff, None);
}

#[test]
fn as_of_cutoff_is_set_only_under_as_of_semantics() {
    // The supersede penalty is scoped by `--as-of` alone; `--until` carries
    // the same cutoff but must leave the penalty at present-day.
    let until = TimeScope {
        cutoff: Some("2026-04-01T00:00:00Z".into()),
        ..TimeScope::none()
    };
    assert_eq!(
        until.as_of_cutoff(),
        None,
        "--until must not scope the penalty"
    );

    let as_of = TimeScope {
        as_of: true,
        ..until.clone()
    };
    assert_eq!(as_of.as_of_cutoff(), Some("2026-04-01T00:00:00Z"));

    assert_eq!(TimeScope::none().as_of_cutoff(), None);
}

#[test]
fn filters_none_constrains_nothing_and_extends_by_field() {
    let none = Filters::none();
    assert_eq!(none.repo, None);
    assert_eq!(none.kind, None);
    assert!(
        none.scope.is_unbounded(),
        "the default filter set carries the unbounded scope"
    );
    assert_eq!(none.window().cutoff, None);

    let narrowed = Filters {
        repo: Some("qwick-backend"),
        kind: Some("decision"),
        ..Filters::none()
    };
    assert_eq!(narrowed.repo, Some("qwick-backend"));
    assert_eq!(narrowed.kind, Some("decision"));
    assert!(narrowed.scope.is_unbounded());

    let scope = TimeScope {
        cutoff: Some("2026-04-01T00:00:00Z".into()),
        ..TimeScope::none()
    };
    let scoped = Filters {
        scope: &scope,
        ..Filters::none()
    };
    assert_eq!(
        scoped.window().cutoff,
        Some("2026-04-01T00:00:00Z"),
        "Filters::window forwards the scope's bounds to the store layer"
    );
}
