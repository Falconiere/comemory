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
/// paragraph for [`chunk::split`] to pack. `flexible(true)` lets the
/// reader accept ragged rows rather than erroring; a row whose field
/// count doesn't match the header count logs a warning so a malformed
/// source row isn't silently invisible.
fn render_rows(reader: &mut csv::Reader<&[u8]>, headers: &[String]) -> Result<String> {
    let mut rendered = String::new();
    for (row_no, result) in reader.records().enumerate() {
        let record = result.map_err(|e| Error::Document(format!("csv row: {e}")))?;
        warn_if_ragged(row_no + 1, headers.len(), record.len());
        let fields = render_fields(headers, &record);
        if fields.is_empty() {
            continue;
        }
        rendered.push_str(&fields.join(", "));
        rendered.push_str("\n\n");
    }
    Ok(rendered)
}

/// Render one row's fields as `header: value` pairs. A field beyond the
/// header list's length — a ragged row with MORE fields than headers —
/// falls back to a positional `field_N` (1-based column) label instead
/// of being silently dropped by a header-length-bounded zip.
fn render_fields(headers: &[String], record: &csv::StringRecord) -> Vec<String> {
    record
        .iter()
        .enumerate()
        .map(|(i, v)| match headers.get(i) {
            Some(h) => format!("{h}: {v}"),
            None => format!("field_{}: {v}", i + 1),
        })
        .collect()
}

/// Warn when a data row's field count doesn't match the header count —
/// `flexible(true)` accepts both a short row (missing trailing fields)
/// and a long row (extra fields, rendered under `field_N` fallback
/// labels) without erroring, so this is the only signal either case
/// leaves behind.
fn warn_if_ragged(row_no: usize, header_count: usize, field_count: usize) {
    if field_count != header_count {
        tracing::warn!(
            row = row_no,
            header_count,
            field_count,
            "delimited: ragged row — field count does not match header count",
        );
    }
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

#[cfg(test)]
#[path = "tests/delimited.rs"]
mod tests;
