//! CSV/TSV extraction via the `csv` crate: the header row labels every
//! field of every data row, rendered one row per line and blank-line
//! separated so [`chunk::split`] packs whole rows per chunk. Named
//! `delimited` (not `csv`) to avoid shadowing the `csv` crate inside this
//! module.

use super::{Block, DocumentFormat, ExtractedDocument, chunk, extract::normalize_text};
use crate::prelude::*;

/// Extract `content` as CSV/TSV: delimiter is sniffed from the first
/// line (more tabs than commas → TSV) since the caller only knows
/// [`DocumentFormat::Delimited`], not which of the two it is. Title is
/// always `file_stem` — spreadsheets carry no natural document title.
pub fn extract(content: &[u8], file_stem: &str) -> Result<ExtractedDocument> {
    let text = normalize_text(content);
    let delimiter = sniff_delimiter(&text);

    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers: Vec<String> = reader
        .headers()
        .map_err(|e| Error::Document(format!("csv header: {e}")))?
        .iter()
        .map(str::to_string)
        .collect();

    let rendered = render_rows(&mut reader, &headers)?;
    let block = Block {
        heading_path: Vec::new(),
        text: rendered,
        char_start: 0,
        line_start: 1,
    };
    Ok(ExtractedDocument {
        title: file_stem.to_string(),
        format: DocumentFormat::Delimited,
        chunks: chunk::split(&[block]),
        links: Vec::new(),
    })
}

/// Render every data row as `header: value, header: value, …`, one row
/// per line, each row followed by a blank line so it stands as its own
/// paragraph for [`chunk::split`] to pack.
fn render_rows(reader: &mut csv::Reader<&[u8]>, headers: &[String]) -> Result<String> {
    let mut rendered = String::new();
    for result in reader.records() {
        let record = result.map_err(|e| Error::Document(format!("csv row: {e}")))?;
        let fields: Vec<String> = headers
            .iter()
            .zip(record.iter())
            .map(|(h, v)| format!("{h}: {v}"))
            .collect();
        if fields.is_empty() {
            continue;
        }
        rendered.push_str(&fields.join(", "));
        rendered.push_str("\n\n");
    }
    Ok(rendered)
}

/// Sniff comma vs. tab delimiter from the first line: whichever
/// character is more frequent wins; a tie (including no header at all)
/// defaults to comma.
fn sniff_delimiter(text: &str) -> u8 {
    let first_line = text.lines().next().unwrap_or("");
    let commas = first_line.matches(',').count();
    let tabs = first_line.matches('\t').count();
    if tabs > commas { b'\t' } else { b',' }
}
