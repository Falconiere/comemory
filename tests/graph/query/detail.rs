//! Tests for `Graph::node_detail`.

use crate::graph_fixture;

#[test]
fn node_detail_for_memory_returns_outbound_edges() {
    let fx = graph_fixture::build();
    let seed = format!("m:{}", fx.primary_id);
    let detail = fx
        .graph
        .node_detail(&seed)
        .expect("detail")
        .expect("detail Some");
    assert_eq!(detail.node.kind, "Memory");
    assert_eq!(detail.node.id, seed);
    assert!(detail.outbound.iter().any(|e| e.edge_kind == "InRepo"));
    assert!(detail.outbound.iter().any(|e| e.edge_kind == "Tagged"));
}

#[test]
fn node_detail_unknown_id_returns_none() {
    let fx = graph_fixture::build();
    let detail = fx.graph.node_detail("m:zzzzzzzz").expect("detail");
    assert!(detail.is_none());
}
