//! Split a document's [`Block`]s into size-bounded [`Chunk`]s: paragraph
//! (blank-line) boundaries are preferred cuts within each
//! [`CHUNK_CHAR_CEILING`]-char window, and every cut short of the block's
//! end carries [`CHUNK_OVERLAP`] chars into the next chunk. A chunk never
//! spans two blocks. Pure and deterministic, split into a PURE
//! offset-packing layer ([`pack`]) and the [`split`] driver — same shape
//! as `crate::ast::chunk`.

use super::{Block, Chunk};
use crate::simhash;

/// Ceiling on a chunk's length, in Unicode characters.
pub const CHUNK_CHAR_CEILING: usize = 2000;
/// Characters carried from the tail of one chunk into the head of the
/// next whenever a chunk's end is short of its block's end.
pub const CHUNK_OVERLAP: usize = 200;

/// Split `blocks` into ordered, contiguous [`Chunk`]s. Blocks whose text
/// is blank are skipped. Ordinals are assigned across the whole returned
/// sequence (not per block).
pub fn split(blocks: &[Block]) -> Vec<Chunk> {
    let mut out = Vec::new();
    for block in blocks {
        if block.text.trim().is_empty() {
            continue;
        }
        let chars: Vec<char> = block.text.chars().collect();
        let boundaries = paragraph_boundaries(&block.text);
        for (start, end) in pack(chars.len(), &boundaries) {
            let text: String = chars[start..end].iter().collect();
            if text.trim().is_empty() {
                continue;
            }
            let line_start = block.line_start + newline_count(&chars, start);
            let line_end = block.line_start + newline_count(&chars, end - 1);
            out.push(Chunk {
                ordinal: 0,
                heading_path: block.heading_path.clone(),
                char_range: (block.char_start + start, block.char_start + end),
                line_range: (line_start, line_end),
                simhash: simhash::of_body(&text),
                text,
            });
        }
    }
    for (i, c) in out.iter_mut().enumerate() {
        c.ordinal = i;
    }
    out
}

/// Cut `[0, len)` into chunks capped at [`CHUNK_CHAR_CEILING`] characters.
/// A chunk only gets cut short of `len` when the remaining text would
/// otherwise bust the ceiling — short input yields exactly one chunk, no
/// gratuitous fragmenting at every paragraph break. When a cut is forced,
/// it prefers a `boundaries` entry inside the ceiling window over a hard
/// cut, and carries [`CHUNK_OVERLAP`] characters into the next window.
/// `search_floor` is the previous chunk's true (pre-overlap) end,
/// monotonic — it stops overlap from re-exposing an already-used boundary
/// and cascading into near-empty chunks. Pure and deterministic — the
/// char-offset counterpart of [`crate::ast::chunk::pack_spans`].
pub fn pack(len: usize, boundaries: &[usize]) -> Vec<(usize, usize)> {
    if len == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut search_floor = 0usize;
    loop {
        let end = if len - start <= CHUNK_CHAR_CEILING {
            len
        } else {
            let hard_end = start + CHUNK_CHAR_CEILING;
            boundaries
                .iter()
                .copied()
                .filter(|&b| b > search_floor && b <= hard_end)
                .max()
                .unwrap_or(hard_end)
        };
        out.push((start, end));
        if end >= len {
            break;
        }
        search_floor = end;
        start = end.saturating_sub(CHUNK_OVERLAP).max(start + 1);
    }
    out
}

/// Char offsets into `text` (Unicode-char-counted, matching
/// [`Chunk::char_range`]) where a new paragraph starts: offset 0, plus
/// the offset right after every blank-line run. Fed to [`pack`] as
/// preferred cut points.
fn paragraph_boundaries(text: &str) -> Vec<usize> {
    let mut bounds = vec![0usize];
    let mut offset = 0usize;
    let mut prev_blank = true;
    for line in text.split('\n') {
        let is_blank = line.trim().is_empty();
        if prev_blank && !is_blank && offset > 0 {
            bounds.push(offset);
        }
        prev_blank = is_blank;
        offset += line.chars().count() + 1;
    }
    bounds
}

/// Number of `\n` characters in `chars[..upto]` — the 0-based line delta
/// of the character at index `upto`, since every `\n` strictly before it
/// pushes it one line further down.
fn newline_count(chars: &[char], upto: usize) -> usize {
    chars[..upto].iter().filter(|&&c| c == '\n').count()
}
