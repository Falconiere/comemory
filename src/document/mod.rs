//! Pure, in-process document extraction (TXT/Markdown/HTML/CSV) and
//! chunking. No schema or store dependency: this module and its children
//! turn already-read bytes into an [`ExtractedDocument`] of size-bounded
//! [`Chunk`]s — the store-facing writer (a later step) owns reading the
//! file, enforcing `indexing.max_file_bytes`, and persisting the result.
//!
//! See `extract` for the dispatcher plus TXT/Markdown, `html` for
//! HTML/XHTML, `delimited` for CSV/TSV, and `chunk` for the shared
//! size-bounded splitter every extractor funnels its [`Block`]s through.

/// Size-bounded splitter shared by every extractor.
pub mod chunk;
/// CSV/TSV extraction via the `csv` crate.
pub mod delimited;
/// Format dispatch plus the TXT/Markdown extractors.
pub mod extract;
/// HTML/XHTML extraction via the `tl` crate.
pub mod html;
/// Per-file index writer: fingerprint skip, extraction, and the
/// one-transaction row replacement that keeps `source_files`/
/// `documents`/`document_chunks`/`document_fts` current.
pub mod writer;

/// Recognized document formats the extraction dispatcher
/// ([`extract::extract`]) switches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentFormat {
    /// Plain text, no structural markup.
    Txt,
    /// Markdown with ATX (`#`) headings.
    Markdown,
    /// HTML or XHTML.
    Html,
    /// CSV or TSV.
    Delimited,
}

/// One ordered passage of an extracted document: a heading- or
/// paragraph-bounded slice of the normalized text, capped at
/// [`chunk::CHUNK_CHAR_CEILING`] Unicode characters with
/// [`chunk::CHUNK_OVERLAP`] characters of overlap carried into the next
/// chunk whenever a cut is forced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Zero-based position of this chunk within its document's chunk list.
    pub ordinal: usize,
    /// Breadcrumb of enclosing headings, outermost first; empty when the
    /// chunk sits above the first heading, or the format has none.
    pub heading_path: Vec<String>,
    /// `(start, end)` Unicode-character offsets into the extractor's
    /// normalized full text, `end` exclusive.
    pub char_range: (usize, usize),
    /// `(start, end)` inclusive 1-based line numbers spanned by the chunk.
    pub line_range: (usize, usize),
    /// The chunk's raw text.
    pub text: String,
    /// 64-bit SimHash of `text` (see [`crate::simhash`]).
    pub simhash: u64,
}

/// One in-process extraction result: a title plus its ordered [`Chunk`]s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedDocument {
    /// Document title: first heading / HTML `<title>`, else the file stem.
    pub title: String,
    /// The format the document was extracted as.
    pub format: DocumentFormat,
    /// Ordered passages covering the normalized text.
    pub chunks: Vec<Chunk>,
    /// Raw Markdown link targets (`[text](target)`) found in the document,
    /// first-mention order, deduplicated, image syntax (`![...]()`)
    /// excluded. Empty for every non-Markdown format. This is a syntactic
    /// scan only — resolving a target to a document (relative-path join,
    /// uniqueness) is `graph::doc_link`'s job, not the extractor's.
    pub links: Vec<String>,
}

/// A heading-bounded unit of normalized text, produced by an extractor
/// and consumed by [`chunk::split`]. `char_start`/`line_start` are
/// positions (Unicode chars / 1-based lines) in the extractor's
/// normalized full text; `text` starts exactly there. Public (not
/// `pub(crate)`) so `tests/` can drive [`chunk::split`] directly with
/// hand-built blocks, the same rationale as `ast::chunk::pack_spans`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// Breadcrumb of headings enclosing this block, outermost first.
    pub heading_path: Vec<String>,
    /// The block's raw text.
    pub text: String,
    /// Unicode-char offset of `text`'s first character in the document's
    /// normalized full text.
    pub char_start: usize,
    /// 1-based line number of `text`'s first line in the document's
    /// normalized full text.
    pub line_start: usize,
}
