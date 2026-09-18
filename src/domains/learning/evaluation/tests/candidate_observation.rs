#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Coverage for the observation contract's own invariants: text bounding never
//! splits a character and always digests the untruncated input, the knob
//! snapshot reports whether decay is frozen, and the canonical digest is
//! stable for equal values and different for different ones.

use comemory::config::Config;
use comemory::domains::learning::evaluation::candidate_observation::{
    BoundedText, OBSERVATION_VERSION, RetrievalKnobs, VectorScenario, canonical_digest,
};

#[test]
fn bounding_truncates_at_a_character_boundary_and_never_mid_character() {
    // Each "é" is two bytes, so a 5-byte bound falls inside the third one.
    let source = "ééé";
    assert_eq!(source.len(), 6);
    let bounded = BoundedText::bound(source, 5);
    assert_eq!(bounded.text, "éé", "the bound snaps DOWN to a boundary");
    assert!(bounded.text.len() <= 5);
    assert!(bounded.truncated);
    assert_eq!(bounded.full_bytes, 6);
}

#[test]
fn the_digest_covers_the_full_text_not_the_truncated_one() {
    let source = "fn alpha() { /* a long body that will be cut */ }";
    let tight = BoundedText::bound(source, 8);
    let whole = BoundedText::bound(source, source.len());
    assert_ne!(tight.text, whole.text, "the two bounds differ");
    assert_eq!(
        tight.sha256, whole.sha256,
        "equal digests mean the same passage was seen under different bounds"
    );
    assert!(
        !whole.truncated,
        "an exact-length bound is not a truncation"
    );
}

#[test]
fn an_unavailable_candidate_still_produces_a_text_record() {
    let empty = BoundedText::unavailable();
    assert_eq!(empty.text, "");
    assert_eq!(empty.full_bytes, 0);
    assert!(!empty.truncated);
    assert_eq!(empty.sha256.len(), 64, "the empty digest is still a digest");
}

#[test]
fn a_zero_bound_yields_empty_text_without_panicking() {
    let bounded = BoundedText::bound("anything", 0);
    assert_eq!(bounded.text, "");
    assert!(bounded.truncated);
    assert_eq!(bounded.full_bytes, 8);
}

#[test]
fn knobs_report_frozen_decay_only_at_zero() {
    let mut cfg = Config::defaults();
    cfg.rank.decay = 0.0;
    assert!(RetrievalKnobs::of(&cfg).decay_frozen());
    cfg.rank.decay = 0.5;
    let live = RetrievalKnobs::of(&cfg);
    assert!(
        !live.decay_frozen(),
        "the shipped default decay is NOT frozen; a run under it depends on reference_time"
    );
    assert_eq!(live.decay, 0.5);
}

#[test]
fn the_knobs_digest_separates_two_configurations_and_repeats_for_one() {
    let mut cfg = Config::defaults();
    let a = canonical_digest(&RetrievalKnobs::of(&cfg)).expect("digest");
    let again = canonical_digest(&RetrievalKnobs::of(&cfg)).expect("digest");
    assert_eq!(a, again, "the same knob set must hash the same every time");
    cfg.rank.mmr_lambda = 0.31;
    let b = canonical_digest(&RetrievalKnobs::of(&cfg)).expect("digest");
    assert_ne!(
        a, b,
        "a moved knob must change the retrieval configuration version"
    );
}

#[test]
fn a_supplied_vector_scenario_records_model_dim_and_a_stable_digest() {
    let vector: Vec<f32> = (0..16).map(|i| i as f32 / 16.0).collect();
    let scenario = VectorScenario::supplied("test:unit", &vector);
    match &scenario {
        VectorScenario::Supplied { model, dim, digest } => {
            assert_eq!(model, "test:unit");
            assert_eq!(*dim, 16);
            assert_eq!(digest.len(), 64);
        }
        VectorScenario::Lexical => panic!("supplied() must not produce Lexical"),
    }
    assert_eq!(
        scenario,
        VectorScenario::supplied("test:unit", &vector),
        "the digest must be reproducible for the same vector"
    );
    let mut nudged = vector.clone();
    nudged[3] += 1.0;
    assert_ne!(scenario, VectorScenario::supplied("test:unit", &nudged));
}

#[test]
fn the_contract_version_is_serialized_as_a_number_consumers_can_refuse() {
    assert_eq!(OBSERVATION_VERSION, 1);
    let json = serde_json::to_string(&VectorScenario::Lexical).expect("serialize");
    assert_eq!(json, r#"{"kind":"lexical"}"#);
}
