#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/cli/search_only.rs`: the one thing that can drift now
//! that the `--only` resolution policy lives in `domains::retrieval::scope`.
//!
//! Before #171 the domain names in a `--only` usage error came from
//! `clap::ValueEnum::to_possible_value`, so they could not disagree with the
//! values clap actually accepts. The policy is transport-neutral now and
//! spells them itself, so a `#[clap(rename_all = ...)]` change or a
//! `#[value(name = ...)]` on [`OnlyDomain`] would silently desync the error
//! text from the real flag values. This pins the two together.

use comemory::cli::search_only::{OnlyDomain, resolve_domains};

/// Every `--only` value clap accepts, in declaration order.
const ALL: &[OnlyDomain] = &[OnlyDomain::Memory, OnlyDomain::Document, OnlyDomain::Code];

#[test]
fn the_usage_error_spells_domains_exactly_as_clap_accepts_them() {
    let from_clap = ALL
        .iter()
        .filter_map(clap::ValueEnum::to_possible_value)
        .map(|v| v.get_name().to_string())
        .collect::<Vec<_>>()
        .join(",");
    assert_eq!(
        from_clap, "memory,document,code",
        "clap's own value names changed — the policy's labels must follow"
    );

    let err = resolve_domains(ALL, None)
        .expect_err("every domain at once names code, which is not searchable yet")
        .to_string();
    assert!(
        err.ends_with(&format!("(got: --only {from_clap})")),
        "the usage error must echo the values clap accepts, got: {err}"
    );
}
