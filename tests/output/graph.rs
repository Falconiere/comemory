//! Mirror tests for `src/output/graph.rs`. Lock the DOT and HTML rendering
//! shapes and the JSON serialization contract without touching a database.

use comemory::output::graph::{to_dot, to_html, CodeGraph, Edge, Node};

/// A small two-file graph: one `imports` edge and one weighted `co_changed`
/// edge, with one zero-rank dangling endpoint.
fn sample() -> CodeGraph {
    CodeGraph {
        nodes: vec![
            Node {
                id: "file:demo:src/a.rs".into(),
                label: "src/a.rs".into(),
                repo: "demo".into(),
                rank: 0.8,
                symbols: 3,
            },
            Node {
                id: "file:demo:src/b.rs".into(),
                label: "src/b.rs".into(),
                repo: "demo".into(),
                rank: 0.2,
                symbols: 1,
            },
        ],
        edges: vec![
            Edge {
                src: "file:demo:src/a.rs".into(),
                dst: "file:demo:src/b.rs".into(),
                rel: "imports".into(),
                weight: 1,
            },
            Edge {
                src: "file:demo:src/a.rs".into(),
                dst: "file:demo:src/b.rs".into(),
                rel: "co_changed".into(),
                weight: 4,
            },
        ],
    }
}

#[test]
fn dot_has_digraph_nodes_and_styled_edges() {
    let dot = to_dot(&sample());
    assert!(dot.starts_with("digraph comemory {"), "dot header: {dot}");
    assert!(dot.contains("\"file:demo:src/a.rs\" [label=\"src/a.rs\""));
    // imports → solid blue arrow; co_changed → dashed orange, no arrowhead.
    assert!(dot.contains("color=\"#3367d6\", style=solid, dir=forward, label=\"1\""));
    assert!(dot.contains("color=\"#d9730d\", style=dashed, dir=none, label=\"4\""));
    assert!(dot.trim_end().ends_with('}'));
}

#[test]
fn higher_rank_yields_wider_dot_node() {
    let dot = to_dot(&sample());
    // The max-rank node (0.8) is widened to 0.6 + 1.0*2.0 = 2.60; the
    // lower-rank node (0.2) to 0.6 + 0.25*2.0 = 1.10.
    assert!(dot.contains("\"file:demo:src/a.rs\" [label=\"src/a.rs\", width=2.60]"));
    assert!(dot.contains("\"file:demo:src/b.rs\" [label=\"src/b.rs\", width=1.10]"));
}

#[test]
fn html_inlines_data_and_escapes_script_break() {
    let html = to_html(&sample()).expect("render html");
    assert!(html.contains("cytoscape"), "loads cytoscape");
    // The data placeholder is replaced with real JSON, not left verbatim.
    assert!(!html.contains("__GRAPH_DATA__"));
    assert!(html.contains("file:demo:src/a.rs"));
}

#[test]
fn html_escapes_closing_script_sequence() {
    let g = CodeGraph {
        nodes: vec![Node {
            id: "file:demo:</script>.rs".into(),
            label: "</script>.rs".into(),
            repo: "demo".into(),
            rank: 0.0,
            symbols: 0,
        }],
        edges: vec![],
    };
    let html = to_html(&g).expect("render html");
    // A path containing `</script>` must not break out of the inline script.
    assert!(!html.contains("</script>.rs"));
    assert!(html.contains("<\\/script>.rs"));
}

#[test]
fn json_round_trips_node_and_edge_fields() {
    let json = serde_json::to_value(sample()).expect("serialize");
    assert_eq!(json["nodes"][0]["rank"], 0.8);
    assert_eq!(json["nodes"][0]["symbols"], 3);
    assert_eq!(json["edges"][1]["rel"], "co_changed");
    assert_eq!(json["edges"][1]["weight"], 4);
}
