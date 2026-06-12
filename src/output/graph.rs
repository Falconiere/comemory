//! Render the file-level code-connection graph (PageRank-weighted nodes +
//! `imports` / `co_changed` edges) in three shapes: machine-readable JSON,
//! Graphviz DOT (`dot -Tsvg`), and an interactive HTML page backed by
//! `sigma.js` + `graphology` (WebGL render, ForceAtlas2 layout, loaded from a
//! CDN, so the page needs network access on first load). The data shape
//! (`CodeGraph`) is built by
//! `crate::cli::graph` and consumed here; keeping the structs in `output`
//! lets the integration tests render without a database.

use std::io::Write as _;

use serde::Serialize;

use crate::output::json;
use crate::prelude::*;

/// One graph node: a single source file, keyed by its canonical graph id
/// (`file:<repo>:<path>`). `rank` is the materialized PageRank projected
/// onto the file; `symbols` counts its top-level (non-chunk) symbols.
#[derive(Serialize, Clone, Debug)]
pub struct Node {
    /// Canonical graph id, `file:<repo>:<path>`.
    pub id: String,
    /// Display label — the repo-relative path.
    pub label: String,
    /// Owning repo label.
    pub repo: String,
    /// Materialized PageRank score (`code_symbols.rank_score`); `0.0` for
    /// files that appear only as a dangling edge endpoint.
    pub rank: f64,
    /// Count of top-level symbols indexed in this file.
    pub symbols: u32,
}

/// One directed graph edge between two file nodes.
#[derive(Serialize, Clone, Debug)]
pub struct Edge {
    /// Source node id (`file:<repo>:<path>`).
    pub src: String,
    /// Destination node id (`file:<repo>:<path>`).
    pub dst: String,
    /// Relation kind: `imports` or `co_changed`.
    pub rel: String,
    /// Edge weight (accumulated co-change count; `1` for imports).
    pub weight: i64,
}

/// The full exportable graph.
#[derive(Serialize, Debug)]
pub struct CodeGraph {
    /// File nodes, sorted by id for deterministic output.
    pub nodes: Vec<Node>,
    /// Directed edges, ordered `(rel, src, dst)`.
    pub edges: Vec<Edge>,
}

/// Embedded HTML viewer template; `__GRAPH_DATA__` is replaced with the
/// inlined JSON payload at render time.
const TEMPLATE: &str = include_str!("graph_template.html");

/// Write the graph as a single line of JSON to stdout.
pub fn write_json(g: &CodeGraph) -> Result<()> {
    json::write(g)
}

/// Write the Graphviz DOT rendering to stdout.
pub fn write_dot(g: &CodeGraph) -> Result<()> {
    let mut out = std::io::stdout().lock();
    write!(out, "{}", to_dot(g))?;
    Ok(())
}

/// Write the interactive HTML viewer to stdout.
pub fn write_html(g: &CodeGraph) -> Result<()> {
    let mut out = std::io::stdout().lock();
    write!(out, "{}", to_html(g)?)?;
    Ok(())
}

/// Render the graph as Graphviz DOT. Node width scales with PageRank;
/// `imports` edges are solid blue arrows, `co_changed` edges dashed orange
/// (undirected) with the weight as the label.
pub fn to_dot(g: &CodeGraph) -> String {
    let max_rank = g.nodes.iter().map(|n| n.rank).fold(0.0_f64, f64::max);
    let mut s = String::from(
        "digraph comemory {\n  rankdir=LR;\n  \
         node [shape=box, style=filled, fillcolor=\"#e8eef9\", fontname=\"monospace\"];\n",
    );
    for n in &g.nodes {
        let scale = if max_rank > 0.0 {
            n.rank / max_rank
        } else {
            0.0
        };
        let width = 0.6 + scale * 2.0;
        s.push_str(&format!(
            "  \"{}\" [label=\"{}\", width={width:.2}];\n",
            dot_escape(&n.id),
            dot_escape(&n.label),
        ));
    }
    for e in &g.edges {
        let (color, style, dir) = match e.rel.as_str() {
            "imports" => ("#3367d6", "solid", "forward"),
            "co_changed" => ("#d9730d", "dashed", "none"),
            // Defensive default; `rels_of` only ever emits the two arms above.
            _ => ("#888888", "solid", "forward"),
        };
        s.push_str(&format!(
            "  \"{}\" -> \"{}\" [color=\"{color}\", style={style}, dir={dir}, label=\"{}\"];\n",
            dot_escape(&e.src),
            dot_escape(&e.dst),
            e.weight,
        ));
    }
    s.push_str("}\n");
    s
}

/// Render the interactive HTML viewer by inlining the graph JSON into the
/// embedded template. The payload's `</` sequences are escaped so a path can
/// never break out of the `<script>` element, and the line-terminator code
/// points U+2028 / U+2029 are escaped to their `\u….` form so pre-ES2019
/// engines can still parse the inlined string literal.
pub fn to_html(g: &CodeGraph) -> Result<String> {
    let data = serde_json::to_string(g)?
        .replace("</", "<\\/")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    Ok(TEMPLATE.replace("__GRAPH_DATA__", &data))
}

/// Escape a string for use inside a double-quoted Graphviz DOT identifier
/// or label. `\` and `"` are the metacharacters that matter; raw newlines
/// (which can legally appear in a POSIX path) are escaped to `\n` / `\r` so
/// they never produce invalid DOT syntax. `\` must be replaced first so the
/// escapes introduced by the later passes are not themselves doubled.
fn dot_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}
