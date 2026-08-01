//! File classification: the v1 extension allowlist (TXT/Markdown/
//! HTML-XHTML/CSV-TSV) plus a binary-content sniff, per the design
//! spec's "Classification and discovery" rule 4. [`Classification`]'s
//! third outcome, `Ignored`, belongs to rule 3 (the managed memory
//! directory exclusion) and is produced by the discovery walk, not by
//! [`classify`] itself — see that variant's doc.

use std::path::Path;

use crate::document::DocumentFormat;

/// How a candidate file under a registered source root resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// A document of the given extractable format (rule 4's allowlist
    /// match, content-sniffed to rule out a misnamed binary).
    Document(DocumentFormat),
    /// An extension outside the v1 allowlist, or an allowlisted
    /// extension whose content sniffed as binary. Recorded for
    /// diagnostics (spec: "Everything else is recorded `unsupported`").
    Unsupported,
    /// Excluded before the allowlist ever runs, by rule 3 (Comemory's
    /// managed memory directory). [`classify`] never returns this
    /// variant — only [`crate::source::discover::discover`] does, for a
    /// single-file source whose registered path falls inside the
    /// managed directory. A directory-source walk instead prunes the
    /// whole managed-directory subtree, so its files never reach
    /// classification at all.
    Ignored,
}

/// Number of leading bytes the discovery walk sniffs before trusting an
/// allowlisted extension — large enough to see past a BOM or leading
/// blank lines, small enough to stay cheap on every file in a walk.
pub const SNIFF_WINDOW: usize = 8192;

/// Classify `path` by its extension against the v1 allowlist (TXT,
/// Markdown, HTML/XHTML, CSV/TSV — spec rule 4), downgrading to
/// [`Classification::Unsupported`] when `content_head` — the file's
/// first bytes, read by the caller — contains a NUL byte, the quick
/// binary sniff that catches a misnamed binary even under an
/// allowlisted extension. `content_head` may be shorter than
/// [`SNIFF_WINDOW`] (a small file); only the bytes given, up to that
/// bound, are inspected.
pub fn classify(path: &Path, content_head: &[u8]) -> Classification {
    let Some(format) = format_of_extension(path) else {
        return Classification::Unsupported;
    };
    let window = &content_head[..content_head.len().min(SNIFF_WINDOW)];
    if window.contains(&0u8) {
        return Classification::Unsupported;
    }
    Classification::Document(format)
}

/// Resolve a [`DocumentFormat`] from `path`'s extension
/// (case-insensitive). `None` for anything outside the v1 allowlist.
fn format_of_extension(path: &Path) -> Option<DocumentFormat> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "txt" => Some(DocumentFormat::Txt),
        "md" | "markdown" => Some(DocumentFormat::Markdown),
        "html" | "htm" | "xhtml" => Some(DocumentFormat::Html),
        "csv" | "tsv" => Some(DocumentFormat::Delimited),
        _ => None,
    }
}
