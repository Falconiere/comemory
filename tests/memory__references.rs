//! Round-trip coverage for the string-or-struct `Ref` (and `References`)
//! serde shape: a bare (unanchored) ref must serialize back to the *same*
//! bare scalar in both YAML and JSON (no `{id: ...}` map), while an anchored
//! ref round-trips as a full `{id, blob, commit, branch}` map.

use comemory::memory::{Ref, References};

/// A bare-string YAML scalar deserializes to an unanchored `Ref` and
/// re-serializes byte-stable to the same bare scalar (no map wrapper).
#[test]
fn bare_string_yaml_round_trips_as_scalar() {
    let yaml = "qwick:src/db.rs:connect\n";
    let r: Ref = serde_yaml::from_str(yaml).expect("deserialize bare yaml");
    assert_eq!(r.id, "qwick:src/db.rs:connect");
    assert_eq!(r.blob, None);
    assert_eq!(r.commit, None);
    assert_eq!(r.branch, None);

    let out = serde_yaml::to_string(&r).expect("serialize bare yaml");
    assert_eq!(
        out, yaml,
        "bare ref must re-serialize as a scalar, not a map"
    );
    assert!(!out.contains("id:"), "no map key should appear: {out}");
}

/// The same byte-stability holds for JSON: a bare JSON string deserializes to
/// an unanchored `Ref` and re-serializes to the same JSON string literal.
#[test]
fn bare_string_json_round_trips_as_string() {
    let json = "\"qwick:src/db.rs\"";
    let r: Ref = serde_json::from_str(json).expect("deserialize bare json");
    assert_eq!(r, Ref::new("qwick:src/db.rs"));

    let out = serde_json::to_string(&r).expect("serialize bare json");
    assert_eq!(out, json, "bare ref must re-serialize as a JSON string");
}

/// A structured `{id, blob, commit, branch}` map round-trips fully in both
/// YAML and JSON with every anchor field preserved.
#[test]
fn structured_map_round_trips_fully() {
    let anchored = Ref {
        id: "qwick:src/db.rs:connect".to_string(),
        blob: Some("a1b2c3d4e5f6".to_string()),
        commit: Some("deadbeefcafe".to_string()),
        branch: Some("main".to_string()),
    };

    let yaml = serde_yaml::to_string(&anchored).expect("yaml ser");
    assert!(yaml.contains("id:"), "anchored ref must serialize as a map");
    let back_yaml: Ref = serde_yaml::from_str(&yaml).expect("yaml de");
    assert_eq!(
        back_yaml, anchored,
        "yaml map round-trip must preserve all anchors"
    );

    let json = serde_json::to_string(&anchored).expect("json ser");
    let back_json: Ref = serde_json::from_str(&json).expect("json de");
    assert_eq!(
        back_json, anchored,
        "json map round-trip must preserve all anchors"
    );
}

/// A map missing the optional anchor fields deserializes to an unanchored
/// `Ref` and then collapses back to a bare scalar on re-serialize.
#[test]
fn map_with_only_id_deserializes_and_reserializes_bare() {
    let r: Ref = serde_yaml::from_str("id: qwick:src/lib.rs\n").expect("de id-only map");
    assert_eq!(r, Ref::new("qwick:src/lib.rs"));
    let out = serde_yaml::to_string(&r).expect("ser id-only");
    assert_eq!(out, "qwick:src/lib.rs\n");
}

/// A `References` mixing a bare file ref with an anchored symbol ref round-trips
/// in YAML: the bare entry stays a scalar, the anchored entry stays a map.
#[test]
fn references_with_mixed_bare_and_anchored_round_trips() {
    let refs = References {
        files: vec![Ref::new("qwick:src/lib.rs")],
        symbols: vec![Ref {
            id: "qwick:src/db.rs:connect".to_string(),
            blob: Some("0011223344".to_string()),
            commit: Some("ffeeddccbb".to_string()),
            branch: Some("feature/x".to_string()),
        }],
    };

    let yaml = serde_yaml::to_string(&refs).expect("yaml ser refs");
    let back: References = serde_yaml::from_str(&yaml).expect("yaml de refs");
    assert_eq!(back, refs, "mixed bare+anchored References must round-trip");

    let json = serde_json::to_string(&refs).expect("json ser refs");
    let back_json: References = serde_json::from_str(&json).expect("json de refs");
    assert_eq!(
        back_json, refs,
        "mixed References must round-trip via JSON too"
    );
}
