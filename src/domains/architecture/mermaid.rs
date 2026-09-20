//! Rendering a model as Mermaid `flowchart` source. The console draws the
//! model with whatever library it likes; this is the terminal's and the
//! markdown reader's view of the same value.
//!
//! Output is deterministic — declaration order throughout — so a diagram
//! pasted into a document only changes when the model does.

use std::fmt::Write as _;

use crate::domains::architecture::model::{Component, EdgeKind, Model};

/// Render `model` as Mermaid `flowchart` source, newline-terminated.
pub fn render(model: &Model) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "flowchart {}", model.direction.as_str());
    for group in &model.groups {
        let _ = writeln!(out, "  subgraph {}[\"{}\"]", group.id, escape(&group.name));
        for c in model.components.iter().filter(|c| in_group(c, &group.id)) {
            let _ = writeln!(out, "    {}", node(c));
        }
        let _ = writeln!(out, "  end");
    }
    for c in model.components.iter().filter(|c| c.group.is_none()) {
        let _ = writeln!(out, "  {}", node(c));
    }
    for e in &model.edges {
        let _ = writeln!(out, "  {} -->|{}| {}", e.from, label(e.kind), e.to);
    }
    out
}

/// Whether `c` belongs to the group `id`.
fn in_group(c: &Component, id: &str) -> bool {
    c.group.as_deref() == Some(id)
}

/// One node declaration, `id["Name"]`.
fn node(c: &Component) -> String {
    format!("{}[\"{}\"]", c.id, escape(&c.name))
}

/// The edge label Mermaid shows on the arrow.
fn label(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Imports => "imports",
        EdgeKind::CoChanged => "co-changed",
        EdgeKind::Calls => "calls",
        EdgeKind::Depends => "depends",
        EdgeKind::Reads => "reads",
        EdgeKind::Writes => "writes",
        EdgeKind::Publishes => "publishes",
    }
}

/// Escape the characters that would end a Mermaid label early. Mermaid reads
/// `#quot;` as a literal quote and `#124;` as a literal pipe.
fn escape(text: &str) -> String {
    text.replace('"', "#quot;").replace('|', "#124;")
}

#[cfg(test)]
#[path = "tests/mermaid.rs"]
mod tests;
