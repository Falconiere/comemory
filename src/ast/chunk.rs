//! cAST-style greedy chunking of oversized AST nodes: pack consecutive
//! child nodes into line-budgeted chunks, recursing into any single
//! child that exceeds the budget alone. Chunks never cut through an
//! AST node.
//!
//! The module is split into a PURE packing layer ([`pack_spans`], hit
//! directly by property tests) and an ast-grep-bound walker
//! ([`chunk_node`]). The walker packs one tree level at a time: any
//! packed chunk that is a single over-budget child is re-chunked by
//! recursing into that child and splicing the result in place —
//! recursive packing per level, rather than flatten-then-pack-once,
//! keeps every chunk boundary aligned to a node boundary at the level
//! where the split happens.
//!
//! Coverage contract: chunks tile the node's span from the line AFTER
//! the node's first line (the signature/header line stays with the
//! parent row — see `cli::index_code`) down to the node's last line.
//! Gap lines BETWEEN packed groups (blank lines; comments are AST
//! children and therefore packed) are dropped from chunk text — an
//! acceptable loss for FTS/embedding purposes.

use ast_grep_core::{Doc, Node};

/// Line budget per chunk; symbols at or under this stay whole.
pub const CHUNK_LINE_BUDGET: usize = 60;
/// Trailing fragments smaller than this merge into the previous chunk.
pub const MIN_CHUNK_LINES: usize = 5;

/// One chunk: an inclusive one-based line span of the original source
/// plus its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// One-based first line.
    pub line_start: usize,
    /// One-based last line (inclusive).
    pub line_end: usize,
    /// Source text sliced by the line span.
    pub text: String,
    /// True when the chunk is a single AST node larger than the budget
    /// whose children could not be split further.
    pub single_node: bool,
}

/// Greedily pack sorted, non-overlapping one-based line spans into
/// budgeted groups, returning `(line_start, line_end, single_node)`
/// triples.
///
/// Rules, in order:
/// 1. A single span wider than [`CHUNK_LINE_BUDGET`] flushes the
///    accumulator and is emitted verbatim with `single_node = true`
///    (recursion into its children happens at the node layer, never
///    here — verbatim bounds are what lets [`chunk_node`] map the
///    chunk back to its child node).
/// 2. Otherwise spans accumulate while the combined span (including
///    any gap lines between them) stays within the budget.
/// 3. After packing, a trailing chunk under [`MIN_CHUNK_LINES`] merges
///    into the previous chunk — unless the previous chunk is a
///    single-node chunk (kept verbatim for recursion) or the merge
///    would push the previous chunk over the budget; in those two
///    cases the small trailing fragment survives as-is.
///
/// This is the pure half of the chunker; it is `pub` (not
/// `pub(crate)`) only so the `tests/` property tests can drive it
/// directly — production callers go through [`chunk_node`].
pub fn pack_spans(spans: &[(usize, usize)]) -> Vec<(usize, usize, bool)> {
    let mut out: Vec<(usize, usize, bool)> = Vec::new();
    let mut acc: Option<(usize, usize)> = None;
    for &(s, e) in spans {
        if e.saturating_sub(s) + 1 > CHUNK_LINE_BUDGET {
            if let Some((a_start, a_end)) = acc.take() {
                out.push((a_start, a_end, false));
            }
            out.push((s, e, true));
        } else if let Some((a_start, a_end)) = acc {
            if e.saturating_sub(a_start) < CHUNK_LINE_BUDGET {
                acc = Some((a_start, e.max(a_end)));
            } else {
                out.push((a_start, a_end, false));
                acc = Some((s, e));
            }
        } else {
            acc = Some((s, e));
        }
    }
    if let Some((a_start, a_end)) = acc {
        out.push((a_start, a_end, false));
    }
    merge_small_trailing(&mut out);
    out
}

/// Apply packing rule 3: fold a sub-[`MIN_CHUNK_LINES`] trailing chunk
/// into its predecessor when that neither corrupts a verbatim
/// single-node chunk nor busts the budget.
fn merge_small_trailing(out: &mut Vec<(usize, usize, bool)>) {
    let n = out.len();
    if n < 2 {
        return;
    }
    let (last_start, last_end, last_single) = out[n - 1];
    let (prev_start, _, prev_single) = out[n - 2];
    if last_single
        || prev_single
        || last_end - last_start + 1 >= MIN_CHUNK_LINES
        || last_end - prev_start + 1 > CHUNK_LINE_BUDGET
    {
        return;
    }
    out[n - 2].1 = last_end;
    out.pop();
}

/// Chunk an oversized matched node into line-budgeted [`Chunk`]s.
///
/// `source` must be the full file text the node was parsed from —
/// node positions are file-absolute, and chunk line spans are sliced
/// out of `source` directly. The node's first line (its signature /
/// header) is excluded: chunks tile `node start + 1 ..= node end`.
/// Returns an empty vec for single-line nodes.
pub fn chunk_node<D: Doc>(node: &Node<'_, D>, source: &str) -> Vec<Chunk> {
    let lines: Vec<&str> = source.lines().collect();
    let (start, end) = line_span(node);
    if end <= start {
        return Vec::new();
    }
    chunk_into(node, &lines, (start + 1, end))
}

/// Recursive worker behind [`chunk_node`]: produce chunks tiling
/// exactly the `clamp` line range using `node`'s direct children as
/// split boundaries. Children include unnamed tokens (braces) and
/// comments, so the only lines not owned by some child are blank
/// inter-child lines.
fn chunk_into<D: Doc>(node: &Node<'_, D>, lines: &[&str], clamp: (usize, usize)) -> Vec<Chunk> {
    let kids = disjoint_child_spans(node, clamp);
    if kids.is_empty() {
        // Leaf (or all children live above the clamp): nothing left to
        // split on, emit the whole range as one chunk. It only carries
        // the single-node marker when it actually exceeds the budget.
        let over = clamp.1 - clamp.0 + 1 > CHUNK_LINE_BUDGET;
        return vec![make_chunk(lines, clamp.0, clamp.1, over)];
    }
    let spans: Vec<(usize, usize)> = kids.iter().map(|(s, e, _)| (*s, *e)).collect();
    let mut out = Vec::new();
    for (s, e, single) in pack_spans(&spans) {
        if single {
            // Map the verbatim single-node chunk back to its child and
            // recurse; splice the sub-chunks in when the child actually
            // split, otherwise keep the marked chunk as-is.
            let child = kids.iter().find(|(ks, ke, _)| *ks == s && *ke == e);
            if let Some((_, _, child)) = child {
                let sub = chunk_into(child, lines, (s, e));
                if sub.len() > 1 {
                    out.extend(sub);
                    continue;
                }
            }
            out.push(make_chunk(lines, s, e, true));
        } else {
            out.push(make_chunk(lines, s, e, false));
        }
    }
    out
}

/// Collect `node`'s direct-children line spans, clamped to `clamp` and
/// made disjoint (a line shared by two consecutive children is owned
/// by the earlier one; children fully above the floor are dropped).
/// The first span is extended back to `clamp.0` and the last forward
/// to `clamp.1` so the spans tile the clamp range exactly — header /
/// trailer lines (e.g. an opening or closing brace already owned by an
/// outer chunk's line) join the nearest child.
fn disjoint_child_spans<'r, D: Doc>(
    node: &Node<'r, D>,
    clamp: (usize, usize),
) -> Vec<(usize, usize, Node<'r, D>)> {
    let mut out: Vec<(usize, usize, Node<'r, D>)> = Vec::new();
    let mut floor = clamp.0;
    for child in node.children() {
        let (s, e) = line_span(&child);
        let s = s.max(floor);
        let e = e.min(clamp.1);
        if e < s {
            continue;
        }
        floor = e + 1;
        out.push((s, e, child));
    }
    if let Some(first) = out.first_mut() {
        first.0 = clamp.0;
    }
    if let Some(last) = out.last_mut() {
        last.1 = clamp.1;
    }
    out
}

/// One-based inclusive line span of `node`. tree-sitter end positions
/// are exclusive: an end sitting at column 0 of line N means the node's
/// content stops at the end of line N-1, so the span must not claim
/// line N.
pub(crate) fn line_span<D: Doc>(node: &Node<'_, D>) -> (usize, usize) {
    let start = node.start_pos().line() + 1;
    let (end_line, end_col) = node.end_pos().byte_point();
    let end = if end_col == 0 && end_line + 1 > start {
        end_line
    } else {
        end_line + 1
    };
    (start, end.max(start))
}

/// Slice `lines` (zero-based storage of one-based line numbers) into a
/// [`Chunk`] spanning `s ..= e`. Out-of-range bounds are clamped so a
/// trailing line miscount can never panic.
fn make_chunk(lines: &[&str], s: usize, e: usize, single_node: bool) -> Chunk {
    let lo = s.saturating_sub(1).min(lines.len());
    let hi = e.min(lines.len()).max(lo);
    Chunk {
        line_start: s,
        line_end: e,
        text: lines[lo..hi].join("\n"),
        single_node,
    }
}
