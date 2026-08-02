//! Dispatch by [`DocumentFormat`], plus the TXT/Markdown extractors:
//! lossy UTF-8 decode, `\r\n`/`\r` normalization, ATX (`#`) heading
//! detection — skipped inside fenced code blocks — building the
//! heading-path breadcrumb, then delegation to [`chunk::split`].

use once_cell::sync::Lazy;
use regex::Regex;

use super::{Block, DocumentFormat, ExtractedDocument, chunk, delimited, html};
use crate::prelude::*;

/// Extract `content` — bytes already read by the caller, not
/// size-checked here (`indexing.max_file_bytes` is the writer's concern)
/// — as `format`, falling back to `file_stem` for the title when no
/// heading / `<title>` is present.
pub fn extract(
    format: DocumentFormat,
    content: &[u8],
    file_stem: &str,
) -> Result<ExtractedDocument> {
    match format {
        DocumentFormat::Txt => Ok(extract_txt(content, file_stem)),
        DocumentFormat::Markdown => Ok(extract_markdown(content, file_stem)),
        DocumentFormat::Html => html::extract(content, file_stem),
        DocumentFormat::Delimited => delimited::extract(content, file_stem),
    }
}

/// Compiled once: an inline Markdown link `[text](target "optional title")`,
/// with the leading `!` captured so [`extract_markdown_links`] can
/// recognize (and skip) image syntax. `.ok()` keeps the type panic-free,
/// matching `graph::cross_link::REF_RE`.
static MD_LINK_RE: Lazy<Option<Regex>> =
    Lazy::new(|| Regex::new(r#"(!)?\[[^\]]*\]\(([^)\s]+)(?:\s+"[^"]*")?\)"#).ok());

/// Scan `text` for inline Markdown links and return every non-image
/// target, first-mention order, deduplicated. `#fragment` suffixes are
/// kept — [`crate::graph::doc_link`] strips them while resolving. Runs
/// over the whole normalized text, fenced code blocks included: an
/// unresolvable target inside a code sample is simply left unresolved by
/// the deriver, never a wrongly-guessed edge.
fn extract_markdown_links(text: &str) -> Vec<String> {
    let Some(re) = MD_LINK_RE.as_ref() else {
        return Vec::new();
    };
    let mut links = Vec::new();
    for cap in re.captures_iter(text) {
        if cap.get(1).is_some() {
            continue; // image syntax `![...]()`
        }
        let Some(target) = cap.get(2) else { continue };
        let target = target.as_str().to_string();
        if !links.contains(&target) {
            links.push(target);
        }
    }
    links
}

/// Decode `content` as lossy UTF-8 and normalize `\r\n`/`\r` line endings
/// to `\n`. Shared by every extractor that reads plain text.
pub(crate) fn normalize_text(content: &[u8]) -> String {
    String::from_utf8_lossy(content)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

/// TXT extraction: one block spanning the whole normalized text, no
/// heading path. Title is always `file_stem` — plain text carries no
/// heading to promote.
fn extract_txt(content: &[u8], file_stem: &str) -> ExtractedDocument {
    let text = normalize_text(content);
    let block = Block {
        heading_path: Vec::new(),
        text,
        char_start: 0,
        line_start: 1,
    };
    ExtractedDocument {
        title: file_stem.to_string(),
        format: DocumentFormat::Txt,
        chunks: chunk::split(&[block]),
        links: Vec::new(),
    }
}

fn extract_markdown(content: &[u8], file_stem: &str) -> ExtractedDocument {
    let text = normalize_text(content);
    let (title, blocks) = markdown_blocks(&text, file_stem);
    ExtractedDocument {
        title,
        format: DocumentFormat::Markdown,
        chunks: chunk::split(&blocks),
        links: extract_markdown_links(&text),
    }
}

/// Split `text` into heading-bounded [`Block`]s, tracking a heading
/// breadcrumb stack via [`MdState::on_heading`]; ATX detection is
/// suspended inside fenced (` ``` `/`~~~`) code blocks. Returns the
/// document title — the first heading found, else `file_stem` —
/// alongside the blocks.
fn markdown_blocks(text: &str, file_stem: &str) -> (String, Vec<Block>) {
    let mut state = MdState::new();
    let mut in_fence = false;
    let mut char_offset = 0usize;

    for (line_no, line) in text.split('\n').enumerate() {
        let line_len = line.chars().count();
        let line_start_char = char_offset;
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
        } else if !in_fence && let Some((level, heading_text)) = parse_atx_heading(line) {
            state.on_heading(level, heading_text, text, line_start_char, line_no + 2);
        }
        char_offset += line_len + 1;
    }

    state.finish(text, file_stem)
}

/// Accumulator driving [`markdown_blocks`]: the heading breadcrumb stack
/// plus the blocks flushed so far and the position where the current
/// (still-open) block began.
struct MdState {
    blocks: Vec<Block>,
    stack: Vec<(usize, String)>,
    heading_path: Vec<String>,
    title: Option<String>,
    start_char: usize,
    start_line: usize,
}

impl MdState {
    fn new() -> Self {
        Self {
            blocks: Vec::new(),
            stack: Vec::new(),
            heading_path: Vec::new(),
            title: None,
            start_char: 0,
            start_line: 1,
        }
    }

    /// Flush the block that ends right before this heading (at
    /// `block_end_char` in `full_text`), then push the heading onto the
    /// breadcrumb stack — popping every entry at `level` or deeper first
    /// — and open the next block starting on `next_line`.
    fn on_heading(
        &mut self,
        level: usize,
        text: String,
        full_text: &str,
        block_end_char: usize,
        next_line: usize,
    ) {
        let block_text: String = full_text
            .chars()
            .skip(self.start_char)
            .take(block_end_char - self.start_char)
            .collect();
        self.blocks.push(Block {
            heading_path: self.heading_path.clone(),
            text: block_text,
            char_start: self.start_char,
            line_start: self.start_line,
        });

        if self.title.is_none() {
            self.title = Some(text.clone());
        }
        while self.stack.last().is_some_and(|(l, _)| *l >= level) {
            self.stack.pop();
        }
        self.stack.push((level, text));
        self.heading_path = self.stack.iter().map(|(_, t)| t.clone()).collect();
        self.start_char = block_end_char;
        self.start_line = next_line;
    }

    /// Flush the trailing block (everything after the last heading, or
    /// the whole document when there was none) and resolve the title.
    fn finish(self, full_text: &str, file_stem: &str) -> (String, Vec<Block>) {
        let total_chars = full_text.chars().count();
        let block_text: String = full_text.chars().skip(self.start_char).collect();
        let mut blocks = self.blocks;
        if !block_text.is_empty() || blocks.is_empty() {
            blocks.push(Block {
                heading_path: self.heading_path,
                text: block_text,
                char_start: self.start_char.min(total_chars),
                line_start: self.start_line,
            });
        }
        (self.title.unwrap_or_else(|| file_stem.to_string()), blocks)
    }
}

/// Parse a single line as an ATX heading (`#` through `######` followed
/// by a space/tab or end of line); returns the level and the heading
/// text with any closing `#`-run stripped. `None` when the line isn't a
/// heading.
fn parse_atx_heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    if !rest.is_empty() && !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    let mut text = rest.trim().to_string();
    while text.ends_with('#') {
        text.pop();
    }
    let text = text.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some((hashes, text))
    }
}
