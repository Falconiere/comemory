#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

use std::path::Path;

use comemory::domains::architecture::mermaid::render;
use comemory::domains::architecture::model::Model;

fn valid_model() -> Model {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/common/fixtures/architecture/valid.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn a_grouped_model_renders_subgraphs_nodes_and_labelled_edges() {
    let out = render(&valid_model());
    assert!(out.starts_with("flowchart LR\n"), "{out}");
    assert!(
        out.contains("  subgraph domains[\"Domain cores\"]\n"),
        "{out}"
    );
    assert!(out.contains("    domains_graph[\"Graph\"]\n"), "{out}");
    assert!(out.contains("  end\n"), "{out}");
    assert!(out.contains("  cli[\"CLI\"]\n"), "{out}");
    assert!(out.contains("  cli -->|imports| domains_graph\n"), "{out}");
    assert!(out.contains("  domains_graph -->|reads| sqlite\n"), "{out}");
    // A grouped component is declared once, inside its subgraph only.
    assert_eq!(out.matches("domains_graph[\"Graph\"]").count(), 1, "{out}");
}

#[test]
fn rendering_is_byte_stable() {
    let model = valid_model();
    assert_eq!(render(&model), render(&model));
}

#[test]
fn a_label_that_would_end_early_is_escaped() {
    let mut model = valid_model();
    model.components[1].name = "CLI \"the\" | surface".into();
    let out = render(&model);
    assert!(
        out.contains("cli[\"CLI #quot;the#quot; #124; surface\"]"),
        "{out}"
    );
}
