#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/retrieval/scope.rs`.

use comemory::retrieval::scope::{Domain, Domains, Filters, TimeScope};

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

#[test]
fn domains_all_contains_every_domain() {
    let all = Domains::all();
    assert!(all.contains(Domain::Memory));
    assert!(all.contains(Domain::Document));
    assert!(all.contains(Domain::Code));
}

#[test]
fn domains_memory_only_excludes_the_other_two() {
    let memory = Domains::memory_only();
    assert!(memory.contains(Domain::Memory));
    assert!(!memory.contains(Domain::Document));
    assert!(!memory.contains(Domain::Code));
}

#[test]
fn domains_of_builds_an_arbitrary_subset() {
    let doc_and_code = Domains::of(&[Domain::Document, Domain::Code]);
    assert!(!doc_and_code.contains(Domain::Memory));
    assert!(doc_and_code.contains(Domain::Document));
    assert!(doc_and_code.contains(Domain::Code));

    let single = Domains::of(&[Domain::Document]);
    assert!(single.contains(Domain::Document));
    assert!(!single.contains(Domain::Memory));
    assert!(!single.contains(Domain::Code));

    let empty = Domains::of(&[]);
    assert!(!empty.contains(Domain::Memory));
    assert!(!empty.contains(Domain::Document));
    assert!(!empty.contains(Domain::Code));
}

#[test]
fn domains_is_copy_and_defaults_to_all() {
    // `Domains` must stay `Copy` — it lives on `Filters`, which is
    // deliberately `Copy` itself (see `scope.rs`'s doc comment on why
    // that derive is load-bearing). Using `mask` again after the "move"
    // below is itself the compile-time proof; the assertions confirm the
    // copy carries the same bits.
    let mask = Domains::of(&[Domain::Memory, Domain::Code]);
    let copied = mask;
    assert_eq!(mask, copied);
    assert!(mask.contains(Domain::Memory));

    assert_eq!(
        Domains::default(),
        Domains::all(),
        "default is every domain"
    );
}

#[test]
fn filters_none_defaults_domains_to_all() {
    assert_eq!(
        Filters::none().domains,
        Domains::all(),
        "an unscoped Filters must not silently exclude a domain"
    );
}
