//! `comemory graph` — export the file-level code-connection graph mined by
//! `index-code` (the `imports` + `co_changed` edges, with nodes weighted by
//! the materialized PageRank `code_symbols.rank_score`) as JSON, Graphviz
//! DOT, or an interactive HTML page (the viewer loads `sigma.js` from a
//! CDN, so rendering the page needs network access on first load).
//!
//! The graph is purely a read over `comemory.db`: it never re-indexes. Run
//! `comemory index-code` first so the `edges` table and `rank_score` are
//! populated. Nodes are files (`file:<repo>:<path>`); edge endpoints that
//! have no `code_symbols` rows (stale edges) still appear, with rank `0`.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};

use crate::cli::pagination::PaginationArgs;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::domains::graph::code_graph::{CodeGraph, GraphPage};
use crate::domains::graph::query::{Rel, build_graph_page};
use crate::output::graph as render;
use crate::output::tty;
use crate::prelude::*;
use crate::store::connection;

const EXAMPLES: &str = "\
Examples:
  # Whole graph as JSON (every indexed repo)
  comemory graph

  # Interactive viewer for one repo
  comemory graph --repo myrepo --format html > graph.html && open graph.html

  # Graphviz DOT, imports only, piped to an SVG
  comemory graph --repo myrepo --rel imports --format dot | dot -Tsvg > graph.svg

  # Drop weak co-change links (accumulated weight < 3)
  comemory graph --rel co-changed --min-weight 3";

/// Output rendering for `comemory graph`.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum Format {
    /// Machine-readable `{ nodes, edges }` JSON.
    Json,
    /// Graphviz DOT source (pipe to `dot`).
    Dot,
    /// Interactive HTML page (sigma.js, loaded from a CDN).
    Html,
}

/// Arguments to `comemory graph`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Restrict to one repo label (as passed to `index-code --repo`).
    #[arg(long)]
    pub repo: Option<String>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Json)]
    pub format: Format,
    /// Which edge relations to include.
    #[arg(long, value_enum, default_value_t = Rel::All)]
    pub rel: Rel,
    /// Drop `co_changed` edges whose accumulated weight is below this floor
    /// (does not affect `imports`, which always carry weight 1). Must be >= 1.
    #[arg(
        long,
        default_value_t = 1,
        value_parser = clap::builder::RangedI64ValueParser::<i64>::new().range(1..)
    )]
    pub min_weight: i64,
    /// `--limit` / `--offset` window over the edges (in `weight DESC, rel,
    /// src, dst` order). Default limit 50; `--limit 0` exports the full graph.
    /// Applies to every format — DOT/HTML viz render only the current window.
    #[command(flatten)]
    pub page: PaginationArgs,
}

/// Build and emit the code-connection graph. The global `--json` flag forces
/// JSON output regardless of `--format`. The `--limit` / `--offset` window is
/// applied to the edges in every format: JSON emits the [`GraphPage`]
/// envelope, while DOT/HTML render only the windowed subgraph (the viz modes
/// show the current page, not the whole graph). A trailing pagination footer
/// is printed on stderr after the DOT/HTML payload so the human export still
/// reports the window without polluting the pipeable stdout.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let conn = connection::open(paths.db_path())?;

    let page = build_graph_page(
        &conn,
        a.repo.as_deref(),
        a.rel,
        a.min_weight,
        a.page.limit,
        a.page.offset,
    )?;

    let fmt = if json_flag { Format::Json } else { a.format };
    // JSON emits the envelope; DOT/HTML render the windowed subgraph and append
    // a window footer on stderr so it never corrupts a `| dot` / `>` pipeline.
    match fmt {
        Format::Json => render::write_json_page(&page),
        Format::Dot | Format::Html => render_viz(fmt, page),
    }
}

/// Render a paginated graph in a viz (`dot`/`html`) format: stream the windowed
/// subgraph to stdout, then write the pagination footer to stderr.
fn render_viz(fmt: Format, page: GraphPage) -> Result<()> {
    let (offset, total) = (page.offset, page.total);
    let graph = CodeGraph {
        nodes: page.nodes,
        edges: page.edges,
    };
    let edge_count = graph.edges.len();
    match fmt {
        Format::Dot => render::write_dot(&graph)?,
        // `Json` is dispatched in `run`; treat any non-Dot here as the HTML viz.
        _ => render::write_html(&graph)?,
    }
    tty::write_page_footer(
        &mut std::io::stderr().lock(),
        edge_count,
        offset,
        Some(total),
    )
}
