//! The filesystem half of a dataset export (#210): the closed owned-name set,
//! the sweep that clears it, the JSONL serialization and the manifest write.
//!
//! The sweep is what makes withholding real rather than conditional. Every
//! name this export may write is removed from the output directory before
//! anything is written, so a `holdout.jsonl` an earlier `--include-holdout`
//! run left behind cannot survive into a run that withholds it. Nothing else
//! in the directory is touched.

use std::path::Path;

use crate::domains::learning::evaluation::dataset_build::{BucketKey, Dataset};
use crate::domains::learning::evaluation::dataset_manifest::{DatasetManifest, FileEntry};
use crate::domains::learning::evaluation::dataset_record::{DatasetRecord, LabelClass, Split};
use crate::prelude::*;
use crate::utilities::digest::sha256_hex;

/// The manifest's file name inside the output directory.
pub const MANIFEST_FILE: &str = "manifest.json";

/// What the manifest records against a holdout file this run did not write.
const WITHHELD: &str = "holdout withheld from this export; re-run with --include-holdout";

/// Every file name this command may write: the closed owned-name set the
/// sweep below clears.
fn owned_names() -> Vec<String> {
    let mut names = vec![MANIFEST_FILE.to_string()];
    for split in Split::all() {
        for labels in [LabelClass::Manual, LabelClass::Implicit] {
            names.push(BucketKey { split, labels }.file_name());
        }
    }
    names
}

/// Clear every owned name, then write each bucket that is not withheld.
/// Returns `(written, withheld)` in that order.
pub fn write_buckets(
    out: &Path,
    dataset: &Dataset,
    include_holdout: bool,
) -> Result<(Vec<FileEntry>, Vec<FileEntry>)> {
    std::fs::create_dir_all(out).map_err(Error::Io)?;
    sweep(out)?;
    let mut written = Vec::new();
    let mut withheld = Vec::new();
    for (key, records) in &dataset.buckets {
        let body = serialize(records)?;
        let entry = FileEntry {
            path: key.file_name(),
            split: key.split,
            labels: key.labels,
            rows: records.len() as u64,
            bytes: body.len() as u64,
            sha256: sha256_hex(body.as_bytes()),
            reason: None,
        };
        if key.split == Split::Holdout && !include_holdout {
            withheld.push(FileEntry {
                reason: Some(WITHHELD.to_string()),
                ..entry
            });
            continue;
        }
        std::fs::write(out.join(&entry.path), body.as_bytes()).map_err(Error::Io)?;
        written.push(entry);
    }
    Ok((written, withheld))
}

/// Write the manifest, last, so an export whose manifest is present is an
/// export whose data files are complete.
pub fn write_manifest(out: &Path, manifest: &DatasetManifest) -> Result<()> {
    let body = serde_json::to_vec_pretty(manifest).map_err(Error::Json)?;
    std::fs::write(out.join(MANIFEST_FILE), body).map_err(Error::Io)
}

/// Remove every file this command owns from `out`.
///
/// A missing name is a no-op; any other removal failure propagates, because a
/// name the export cannot clear is a name it cannot honestly claim to own.
fn sweep(out: &Path) -> Result<()> {
    for name in owned_names() {
        match std::fs::remove_file(out.join(&name)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::Io(e)),
        }
    }
    Ok(())
}

/// One bucket as JSONL: one compact record per line, newline-terminated.
fn serialize(records: &[DatasetRecord]) -> Result<String> {
    let mut body = String::new();
    for record in records {
        body.push_str(&serde_json::to_string(record).map_err(Error::Json)?);
        body.push('\n');
    }
    Ok(body)
}
