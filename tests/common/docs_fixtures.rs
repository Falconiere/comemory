#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Shared helper: copy the real document fixtures
//! (`tests/common/fixtures/docs/`) into a fresh directory so `comemory
//! index`/`sources`/`unindex` integration tests never point at — and can
//! never accidentally mutate — the checked-in originals.
//!
//! Not registered in `common/mod.rs`: each consuming test binary
//! `#[path]`-includes this file directly, matching the `git_repo.rs` /
//! `git_commit.rs` convention, so a binary that doesn't need the docs
//! fixtures never pulls in an unused module.

use std::fs;
use std::path::{Path, PathBuf};

const GUIDE_MD: &[u8] = include_bytes!("fixtures/docs/guide.md");
const CHANGELOG_TXT: &[u8] = include_bytes!("fixtures/docs/changelog.txt");
const PAGE_HTML: &[u8] = include_bytes!("fixtures/docs/page.html");
const DATA_CSV: &[u8] = include_bytes!("fixtures/docs/data.csv");

/// Number of real fixtures [`seed`] writes — every one format-classifies
/// as a document (txt/md/html/csv), so a full first-time index run
/// always reports this many candidates.
pub const FIXTURE_COUNT: usize = 4;

/// Write all four real document fixtures into `<parent>/docs/` and
/// return that directory.
pub fn seed(parent: &Path) -> PathBuf {
    let dir = parent.join("docs");
    fs::create_dir_all(&dir).expect("create docs dir");
    fs::write(dir.join("guide.md"), GUIDE_MD).expect("write guide.md");
    fs::write(dir.join("changelog.txt"), CHANGELOG_TXT).expect("write changelog.txt");
    fs::write(dir.join("page.html"), PAGE_HTML).expect("write page.html");
    fs::write(dir.join("data.csv"), DATA_CSV).expect("write data.csv");
    dir
}
