//! Integration tests for the `output` module. Includes a stable JSON snapshot
//! to catch accidental format drift, plus the tests-mirror entry points for
//! every `src/output/*.rs` file so the `tests-mirror-check` gate is
//! satisfied.

use serde::Serialize;

#[path = "output/tty.rs"]
mod tty;

#[path = "output/json.rs"]
mod json;

#[path = "output/search.rs"]
mod search;

#[path = "output/context.rs"]
mod context;

#[derive(Serialize)]
struct Hit {
    id: &'static str,
    score: f32,
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
