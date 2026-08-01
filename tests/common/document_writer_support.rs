//! Shared support for the `src/document/writer.rs` and
//! `src/document/fingerprint.rs` test mirrors: only the pieces BOTH
//! consuming binaries actually call — opening a disposable
//! `comemory.db` seeded with one `source_roots` row, copying a real
//! document fixture into a temp directory (the writer must never be
//! handed the checked-in fixtures directly since some cases mutate
//! content and permissions), and driving `writer::update_file` for one
//! candidate. Binary-specific extras (e.g. the `guide.md`/`page.html`
//! fixtures, chunk/fts counting, mtime bumping) stay local to whichever
//! test file actually uses them, matching the `cli_rebuild_support.rs`
//! convention of a shared file with no dead code in either consumer.
//!
//! Not registered in `common/mod.rs`: each consuming test binary
//! `#[path]`-includes this file directly.

use std::fs;
use std::path::{Path, PathBuf};

use comemory::document::DocumentFormat;
use comemory::document::writer::{self, UpdateOutcome};
use comemory::source::classify::Classification;
use comemory::source::discover::Candidate;
use comemory::store::sources::SourceRootUpsert;
use comemory::store::{connection, sources};
use rusqlite::Connection;
use tempfile::TempDir;

/// Real changelog fixture — the one document both the writer and the
/// fingerprint mirrors index.
pub const CHANGELOG_TXT: &[u8] = include_bytes!("fixtures/docs/changelog.txt");

pub const SOURCE_ID: &str = "source-under-test";
pub const MAX_BYTES: u64 = 16 * 1024 * 1024;

/// Open a fresh `comemory.db` and seed the `source_roots` row
/// [`SOURCE_ID`] every `source_files` row here is foreign-keyed to.
pub fn open_db(tmp: &TempDir) -> Connection {
    let conn = connection::open(tmp.path().join("comemory.db")).expect("open db");
    sources::upsert(
        &conn,
        SourceRootUpsert {
            id: SOURCE_ID,
            canonical_path: "/does/not/matter",
            kind: "dir",
            repo: None,
            created_at: "2026-01-01T00:00:00.000000000Z",
            updated_at: "2026-01-01T00:00:00.000000000Z",
        },
    )
    .expect("seed source_roots row");
    conn
}

pub fn write_fixture(tmp: &TempDir, name: &str, bytes: &[u8]) -> PathBuf {
    let path = tmp.path().join(name);
    fs::write(&path, bytes).expect("write fixture copy");
    path
}

pub fn candidate(relative: &str, absolute: &Path, format: DocumentFormat) -> Candidate {
    Candidate {
        relative_path: PathBuf::from(relative),
        absolute_path: absolute.to_path_buf(),
        classification: Classification::Document(format),
    }
}

pub fn index(
    conn: &mut Connection,
    path: &Path,
    relative: &str,
    format: DocumentFormat,
) -> UpdateOutcome {
    let c = candidate(relative, path, format);
    let source_root = canonical_parent(path);
    writer::update_file(conn, SOURCE_ID, None, &c, &source_root, MAX_BYTES).expect("update_file")
}

/// Canonicalize `path`'s parent directory — every fixture here is
/// written flat directly under the tempdir root via [`write_fixture`],
/// so this IS that root, resolved the same way `cli::index` resolves a
/// registered source root before handing it to the writer as its TOCTOU
/// boundary.
fn canonical_parent(path: &Path) -> PathBuf {
    fs::canonicalize(path.parent().expect("candidate path has a parent"))
        .expect("canonicalize source root")
}
