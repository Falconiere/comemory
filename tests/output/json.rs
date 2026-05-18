//! Mirror tests for `src/output/json.rs`. The stable JSON snapshot covering
//! the public `write` path lives in `tests/output.rs`
//! (`json_round_trip_is_stable`); this module exists to satisfy the
//! tests-mirror gate and to lock in that `qwick::output::json::write` is
//! callable with a basic `Serialize` value without panicking.

use qwick::output::json;

#[test]
fn write_accepts_serializable_value() {
    // Smoke test: writing to the real stdout via the helper must not error
    // for a trivial payload. The stable JSON shape is asserted via insta in
    // `tests/output.rs::json_round_trip_is_stable`.
    let payload = serde_json::json!({ "ok": true });
    json::write(&payload).expect("json::write must succeed for trivial payload");
}
