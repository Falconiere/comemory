//! Tests for `Graph::expand_neighbors`.

use crate::graph_fixture;

#[test]
fn expand_returns_one_hop_for_known_memory() {
    let fx = graph_fixture::build();
    let seed = format!("m:{}", fx.primary_id);
    let payload = fx.graph.expand_neighbors(&seed, 1).expect("expand");
    let ids: std::collections::BTreeSet<_> = payload.nodes.iter().map(|n| n.id.clone()).collect();
    assert!(ids.contains(&seed));
    assert!(ids.contains("r:qwick-backend"));
    assert!(ids.contains("a:falconiere"));
    assert!(ids.contains("t:database") || ids.contains("t:postgres"));
}

#[test]
fn expand_unknown_id_returns_empty_payload() {
    let fx = graph_fixture::build();
    let payload = fx.graph.expand_neighbors("m:zzzzzzzz", 1).expect("expand");
    assert!(payload.nodes.is_empty());
    assert!(payload.edges.is_empty());
}
