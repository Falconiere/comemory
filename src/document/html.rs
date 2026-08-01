//! HTML/XHTML extraction via the `tl` crate: a recursive DOM walk builds
//! a synthetic "extracted text" buffer (script/style content dropped),
//! headings become the heading-path breadcrumb, `<title>` (else the
//! first heading) becomes the title. Char/line positions in the returned
//! [`Block`]s are offsets into that synthetic buffer, not the original
//! HTML source — there is no meaningful source mapping once tags are
//! stripped.

use super::{Block, DocumentFormat, ExtractedDocument, chunk, extract::normalize_text};
use crate::prelude::*;

/// Tags whose content is dropped entirely (never becomes body text).
const SKIP_TAGS: &[&str] = &["script", "style"];
/// Block-level tags after which a paragraph boundary is inserted so
/// [`chunk::split`] can prefer cutting there.
const BOUNDARY_TAGS: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "header",
    "footer",
    "nav",
    "main",
    "li",
    "ul",
    "ol",
    "table",
    "tr",
    "td",
    "th",
    "pre",
    "blockquote",
    "br",
    "hr",
];

/// Extract `content` as HTML/XHTML: tag-strip to text (script/style
/// dropped), `<title>` or the first heading becomes the title, headings
/// build the heading-path breadcrumb.
pub fn extract(content: &[u8], file_stem: &str) -> Result<ExtractedDocument> {
    let text = normalize_text(content);
    let dom = tl::parse(&text, tl::ParserOptions::default())
        .map_err(|e| Error::Document(format!("html: {e}")))?;
    let parser = dom.parser();

    let mut w = Walker::new();
    for handle in dom.children() {
        walk(handle, parser, &mut w);
    }
    w.flush_block();

    let title = w
        .html_title
        .or(w.first_heading)
        .unwrap_or_else(|| file_stem.to_string());
    Ok(ExtractedDocument {
        title,
        format: DocumentFormat::Html,
        chunks: chunk::split(&w.blocks),
        links: Vec::new(),
    })
}

fn heading_level(name: &str) -> Option<usize> {
    match name {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}

/// Collapse runs of whitespace (including newlines from pretty-printed
/// markup) into single spaces — used for heading/title text only; body
/// text is kept raw.
fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Recursively visit `handle`'s subtree, feeding text runs and headings
/// into `w`. Script/style subtrees are skipped outright; a heading tag
/// short-circuits (its full nested text becomes the heading itself, never
/// body text); every other tag recurses into its direct children and, for
/// [`BOUNDARY_TAGS`], appends a paragraph break on the way out.
fn walk(handle: &tl::NodeHandle, parser: &tl::Parser<'_>, w: &mut Walker) {
    let Some(node) = handle.get(parser) else {
        return;
    };
    match node {
        tl::Node::Tag(tag) => {
            let name = tag.name().as_utf8_str().to_ascii_lowercase();
            if SKIP_TAGS.contains(&name.as_str()) {
                return;
            }
            if let Some(level) = heading_level(&name) {
                let text = collapse_whitespace(&tag.inner_text(parser));
                if !text.is_empty() {
                    w.on_heading(level, text);
                }
                return;
            }
            if name == "title" {
                if w.html_title.is_none() {
                    let text = collapse_whitespace(&tag.inner_text(parser));
                    if !text.is_empty() {
                        w.html_title = Some(text);
                    }
                }
                return;
            }
            for child in tag.children().top().iter() {
                walk(child, parser, w);
            }
            if BOUNDARY_TAGS.contains(&name.as_str()) {
                w.push_text("\n\n");
            }
        }
        tl::Node::Raw(bytes) => w.push_text(&bytes.as_utf8_str()),
        tl::Node::Comment(_) => {}
    }
}

/// Accumulator driving [`walk`]: a growing "extracted text" buffer plus
/// the heading breadcrumb stack and the blocks flushed so far.
struct Walker {
    output: String,
    char_offset: usize,
    line_offset: usize,
    stack: Vec<(usize, String)>,
    heading_path: Vec<String>,
    blocks: Vec<Block>,
    block_start_char: usize,
    block_start_line: usize,
    html_title: Option<String>,
    first_heading: Option<String>,
}

impl Walker {
    fn new() -> Self {
        Self {
            output: String::new(),
            char_offset: 0,
            line_offset: 1,
            stack: Vec::new(),
            heading_path: Vec::new(),
            blocks: Vec::new(),
            block_start_char: 0,
            block_start_line: 1,
            html_title: None,
            first_heading: None,
        }
    }

    fn push_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.char_offset += text.chars().count();
        self.line_offset += text.matches('\n').count();
        self.output.push_str(text);
    }

    /// Flush the block open since the last heading (or the start of the
    /// document), then pop the breadcrumb stack down to `level` and push
    /// this heading, opening the next block right after it.
    fn on_heading(&mut self, level: usize, text: String) {
        self.flush_block();
        if self.first_heading.is_none() {
            self.first_heading = Some(text.clone());
        }
        while self.stack.last().is_some_and(|(l, _)| *l >= level) {
            self.stack.pop();
        }
        self.stack.push((level, text));
        self.heading_path = self.stack.iter().map(|(_, t)| t.clone()).collect();
        self.block_start_char = self.char_offset;
        self.block_start_line = self.line_offset;
    }

    fn flush_block(&mut self) {
        if self.char_offset <= self.block_start_char {
            return;
        }
        let text: String = self
            .output
            .chars()
            .skip(self.block_start_char)
            .take(self.char_offset - self.block_start_char)
            .collect();
        self.blocks.push(Block {
            heading_path: self.heading_path.clone(),
            text,
            char_start: self.block_start_char,
            line_start: self.block_start_line,
        });
    }
}
