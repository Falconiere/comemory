use qwick_memory::retrieval::{Bundle, CitedHit};

#[test]
fn bundle_serializes_to_expected_shape() {
    // Use exactly-representable f32 values (binary fractions) so the
    // serde_json round-trip stays bit-stable across platforms.
    let bundle = Bundle {
        query: "postgres migration".into(),
        route: "Hybrid".into(),
        hits: vec![CitedHit {
            id: "abc123".into(),
            score: 0.5,
            kind: "decision".into(),
            repo: "qwick".into(),
            snippet: "Use Postgres for analytics".into(),
            why: "vector top-1".into(),
        }],
        confidence: 0.25,
        fallback_used: false,
    };

    let v: serde_json::Value = serde_json::to_value(&bundle).unwrap();
    assert_eq!(v["query"], "postgres migration");
    assert_eq!(v["route"], "Hybrid");
    assert_eq!(v["confidence"].as_f64().unwrap(), 0.25);
    assert_eq!(v["fallback_used"], false);
    let hits = v["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["id"], "abc123");
    assert_eq!(hits[0]["score"].as_f64().unwrap(), 0.5);
    assert_eq!(hits[0]["kind"], "decision");
    assert_eq!(hits[0]["repo"], "qwick");
    assert_eq!(hits[0]["snippet"], "Use Postgres for analytics");
    assert_eq!(hits[0]["why"], "vector top-1");
}

#[test]
fn bundle_with_no_hits_still_serializes() {
    let bundle = Bundle {
        query: "x".into(),
        route: "FtsFirst".into(),
        hits: Vec::new(),
        confidence: 0.0,
        fallback_used: true,
    };
    let s = serde_json::to_string(&bundle).unwrap();
    assert!(s.contains("\"hits\":[]"));
    assert!(s.contains("\"fallback_used\":true"));
}
