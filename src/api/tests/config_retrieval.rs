#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/config_retrieval.rs`: the knob projection with
//! its declared ranges, and the validate-then-write update against a real
//! `config.toml` (AC-14's api-layer half — the HTTP half, including the
//! `AppState.cfg` reload, lives in `src/serve/routes/tests/config.rs`).

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::errors::Error;
use tempfile::TempDir;

/// A data-dir with the tree in place but no database — every function here
/// is conn-free and must stay that way.
fn paths_in(home: &TempDir) -> Paths {
    let paths = Paths::new(home.path().join(".comemory"));
    paths.ensure_dirs().expect("dirs");
    paths
}

/// The `UpdateRequest` with everything unset, so each test names only the
/// knob it is exercising.
fn empty_update() -> api::config_retrieval::UpdateRequest {
    api::config_retrieval::UpdateRequest::default()
}

#[test]
fn get_projects_the_live_knobs_and_declares_every_range() {
    let cfg = Config::defaults();
    let knobs = api::config_retrieval::get(&cfg);

    assert_eq!(knobs.rrf_k, cfg.retrieval.rrf_k);
    assert_eq!(knobs.decay, cfg.rank.decay);
    assert_eq!(knobs.mmr_lambda, cfg.rank.mmr_lambda);
    assert_eq!(knobs.bm25_weights, cfg.retrieval.bm25_weights);
    assert_eq!(knobs.graph_hops, cfg.retrieval.graph_hops);
    assert_eq!(knobs.graph_seeds, cfg.retrieval.graph_seeds);
    assert_eq!(knobs.memory_threshold, cfg.retrieval.memory_threshold);
    assert_eq!(knobs.code_threshold, cfg.retrieval.code_threshold);
    assert_eq!(knobs.top_k, cfg.retrieval.top_k);
    assert_eq!(knobs.document_leg_weight, cfg.retrieval.document_leg_weight);
    assert_eq!(knobs.prior_clamp, cfg.rank.prior_clamp);

    // One range per writable knob, mirroring `config::validate`.
    for name in [
        "rrf_k",
        "decay",
        "mmr_lambda",
        "bm25_weights",
        "graph_hops",
        "graph_seeds",
        "memory_threshold",
        "code_threshold",
        "top_k",
        "document_leg_weight",
        "prior_clamp",
    ] {
        assert!(knobs.ranges.contains_key(name), "missing range for {name}");
    }
    let hops = &knobs.ranges["graph_hops"];
    assert_eq!(hops.min, Some(0.0));
    assert_eq!(hops.max, Some(4.0), "the validator caps graph_hops at 4");
    assert_eq!(knobs.ranges["mmr_lambda"].max, Some(1.0));
}

#[test]
fn update_persists_only_the_supplied_keys_and_returns_the_new_knobs() {
    let home = TempDir::new().expect("tempdir");
    let paths = paths_in(&home);
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let knobs = api::config_retrieval::update(
        &mut ctx,
        api::config_retrieval::UpdateRequest {
            rrf_k: Some(30.0),
            graph_hops: Some(1),
            prior_clamp: Some((0.4, 3.0)),
            ..empty_update()
        },
    )
    .expect("update");
    assert_eq!(knobs.rrf_k, 30.0);
    assert_eq!(knobs.graph_hops, 1);
    assert_eq!(knobs.prior_clamp, (0.4, 3.0));
    assert_eq!(
        knobs.top_k, cfg.retrieval.top_k,
        "an unsupplied knob keeps its live value"
    );

    let written = std::fs::read_to_string(paths.config_file()).expect("config.toml");
    assert!(written.contains("rrf_k = 30.0"), "got:\n{written}");
    assert!(written.contains("graph_hops = 1"), "got:\n{written}");
    assert!(
        !written.contains("top_k"),
        "an unsupplied knob must not be materialized:\n{written}"
    );

    // The file is the real source of truth on the next load.
    let reloaded = Config::defaults()
        .with_file(&paths.config_file())
        .expect("reload config.toml");
    assert_eq!(reloaded.retrieval.rrf_k, 30.0);
    assert_eq!(reloaded.retrieval.graph_hops, 1);
    assert_eq!(reloaded.rank.prior_clamp, (0.4, 3.0));

    assert!(
        !paths.db_path().exists(),
        "a config write must not create comemory.db"
    );
}

#[test]
fn an_out_of_range_knob_is_a_bad_request_and_leaves_the_file_byte_identical() {
    let home = TempDir::new().expect("tempdir");
    let paths = paths_in(&home);
    let original = "[retrieval]\nrrf_k = 45.0\n";
    std::fs::write(paths.config_file(), original).expect("seed config.toml");

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let err = api::config_retrieval::update(
        &mut ctx,
        api::config_retrieval::UpdateRequest {
            rrf_k: Some(0.0),
            ..empty_update()
        },
    )
    .expect_err("rrf_k = 0 is rejected by Config::validate");
    assert!(matches!(err, Error::BadRequest(_)), "got {err:?}");
    assert!(
        err.to_string().contains("rrf_k"),
        "the validator's own message must survive: {err}"
    );

    let after = std::fs::read_to_string(paths.config_file()).expect("config.toml");
    assert_eq!(after, original, "a rejected update writes nothing at all");
}

#[test]
fn every_validated_bound_is_reachable_through_the_update_path() {
    let home = TempDir::new().expect("tempdir");
    let paths = paths_in(&home);
    let cfg = Config::defaults();

    for (label, req) in [
        (
            "graph_hops > 4",
            api::config_retrieval::UpdateRequest {
                graph_hops: Some(5),
                ..empty_update()
            },
        ),
        (
            "graph_seeds = 0",
            api::config_retrieval::UpdateRequest {
                graph_seeds: Some(0),
                ..empty_update()
            },
        ),
        (
            "mmr_lambda > 1",
            api::config_retrieval::UpdateRequest {
                mmr_lambda: Some(1.5),
                ..empty_update()
            },
        ),
        (
            "decay < 0",
            api::config_retrieval::UpdateRequest {
                decay: Some(-0.1),
                ..empty_update()
            },
        ),
        (
            "document_leg_weight = 0",
            api::config_retrieval::UpdateRequest {
                document_leg_weight: Some(0.0),
                ..empty_update()
            },
        ),
        (
            "prior_clamp lo > hi",
            api::config_retrieval::UpdateRequest {
                prior_clamp: Some((3.0, 1.0)),
                ..empty_update()
            },
        ),
        (
            "bm25_weights all zero",
            api::config_retrieval::UpdateRequest {
                bm25_weights: Some((0.0, 0.0)),
                ..empty_update()
            },
        ),
    ] {
        let mut ctx = Ctx::lazy(&paths, &cfg);
        let Err(err) = api::config_retrieval::update(&mut ctx, req) else {
            panic!("{label} must be rejected")
        };
        assert!(matches!(err, Error::BadRequest(_)), "{label}: got {err:?}");
        assert!(
            !paths.config_file().exists(),
            "{label}: a rejected update must not create config.toml"
        );
    }
}
