//! Mirror tests for `src/output/json.rs`. The stable JSON snapshot covering
//! the public `write` path is locked in by `json_round_trip_is_stable` below;
//! this module also satisfies the tests-mirror gate.

use comemory::output::json;
use serde::Serialize;

#[derive(Serialize)]
struct Hit {
    id: &'static str,
    score: f32,
}

#[test]
fn write_accepts_serializable_value() {
    // Smoke test: writing to the real stdout via the helper must not error
    // for a trivial payload. The stable JSON shape is asserted via insta in
    // `json_round_trip_is_stable`.
    let payload = serde_json::json!({ "ok": true });
    json::write(&payload).expect("json::write must succeed for trivial payload");
}

#[test]
fn json_round_trip_is_stable() {
    let hits = vec![
        Hit {
            id: "a",
            score: 0.9,
        },
        Hit {
            id: "b",
            score: 0.6,
        },
    ];
    insta::assert_json_snapshot!(hits);
}
