//! Test mirror for `src/source/discover.rs`.

use std::collections::BTreeSet;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use comemory::config::paths::Paths;
use comemory::document::DocumentFormat;
use comemory::source::SourceKind;
use comemory::source::classify::Classification;
use comemory::source::discover::discover;
use tempfile::TempDir;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/common/fixtures/docs");

fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

/// A fresh temp root plus a canonicalized (but otherwise untouched)
/// `memories_dir` sibling — discover must exclude it regardless of what
/// it contains.
fn sandbox() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().expect("tempdir");
    let paths = Paths::new(tmp.path().join(".comemory"));
    paths.ensure_dirs().expect("ensure_dirs");
    let memories_dir = paths
        .memories_dir()
        .canonicalize()
        .expect("canonicalize memories_dir");
    (tmp, memories_dir)
}

fn rel_names(found: &[comemory::source::discover::Candidate]) -> Vec<String> {
    found
        .iter()
        .map(|c| c.relative_path.to_string_lossy().to_string())
        .collect()
}

/// `(fixture name, expected classification)` pairs this test copies AND
/// checks — a single source of truth so covering a fixture added later
/// to `tests/common/fixtures/docs/` is a one-line change here, never two
/// independently-maintained lists drifting apart.
const ALLOWLISTED_FIXTURES: &[(&str, Classification)] = &[
    (
        "changelog.txt",
        Classification::Document(DocumentFormat::Txt),
    ),
    (
        "data.csv",
        Classification::Document(DocumentFormat::Delimited),
    ),
    (
        "guide.md",
        Classification::Document(DocumentFormat::Markdown),
    ),
    ("page.html", Classification::Document(DocumentFormat::Html)),
];

#[test]
fn each_allowlisted_fixture_classifies_correctly() {
    let (tmp, memories_dir) = sandbox();
    let root = tmp.path().join("docs");
    fs::create_dir_all(&root).expect("mkdir root");
    for (name, _) in ALLOWLISTED_FIXTURES {
        fs::copy(fixture(name), root.join(name)).expect("copy fixture");
    }
    let root = root.canonicalize().expect("canonicalize root");

    let mut found = discover(&root, SourceKind::Dir, &memories_dir);
    found.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    let got: Vec<(String, Classification)> = found
        .into_iter()
        .map(|c| {
            (
                c.relative_path.to_string_lossy().to_string(),
                c.classification,
            )
        })
        .collect();

    let mut expected: Vec<(String, Classification)> = ALLOWLISTED_FIXTURES
        .iter()
        .map(|(name, cls)| (name.to_string(), *cls))
        .collect();
    expected.sort_by(|a, b| a.0.cmp(&b.0));

    assert_eq!(got, expected);
}

#[test]
fn deterministic_order_matches_sorted_relative_paths() {
    let (tmp, memories_dir) = sandbox();
    let root = tmp.path().join("docs");
    fs::create_dir_all(root.join("z")).expect("mkdir z");
    fs::create_dir_all(root.join("a")).expect("mkdir a");
    fs::copy(fixture("guide.md"), root.join("z/guide.md")).expect("copy z");
    fs::copy(fixture("changelog.txt"), root.join("a/changelog.txt")).expect("copy a");
    fs::copy(fixture("data.csv"), root.join("data.csv")).expect("copy top");
    let root = root.canonicalize().expect("canonicalize root");

    let found = discover(&root, SourceKind::Dir, &memories_dir);
    let rels = rel_names(&found);
    let mut sorted = rels.clone();
    sorted.sort();

    assert_eq!(rels, sorted, "discover must already return sorted order");
    assert_eq!(rels.len(), 3);
}

#[test]
fn hidden_files_are_excluded() {
    let (tmp, memories_dir) = sandbox();
    let root = tmp.path().join("docs");
    fs::create_dir_all(&root).expect("mkdir");
    fs::copy(fixture("guide.md"), root.join("guide.md")).expect("copy visible");
    fs::copy(fixture("changelog.txt"), root.join(".hidden.txt")).expect("copy hidden");
    let root = root.canonicalize().expect("canonicalize root");

    let found = discover(&root, SourceKind::Dir, &memories_dir);
    assert_eq!(rel_names(&found), vec!["guide.md".to_string()]);
}

#[test]
fn comemoryignore_negation_reincludes_a_pattern() {
    let (tmp, memories_dir) = sandbox();
    let root = tmp.path().join("docs");
    fs::create_dir_all(root.join("drafts")).expect("mkdir drafts");
    fs::copy(fixture("guide.md"), root.join("drafts/guide.md")).expect("copy a");
    fs::copy(fixture("changelog.txt"), root.join("drafts/changelog.txt")).expect("copy b");
    fs::write(root.join(".comemoryignore"), "drafts/*\n!drafts/guide.md\n")
        .expect("write ignore file");
    let root = root.canonicalize().expect("canonicalize root");

    let found = discover(&root, SourceKind::Dir, &memories_dir);
    assert_eq!(rel_names(&found), vec!["drafts/guide.md".to_string()]);
}

/// Isolates rule 3 (managed-directory exclusion) from rule 2's hidden-file
/// default by using a non-dot-prefixed data-dir name, so `memories/`
/// itself is not already excluded as a hidden path.
#[test]
fn managed_memories_dir_is_excluded_from_a_directory_source() {
    let tmp = TempDir::new().expect("tempdir");
    let paths = Paths::new(tmp.path().join("data"));
    paths.ensure_dirs().expect("ensure_dirs");
    let memories_dir = paths
        .memories_dir()
        .canonicalize()
        .expect("canonicalize memories_dir");
    fs::copy(fixture("guide.md"), memories_dir.join("existing-memory.md"))
        .expect("seed a memory file");
    fs::copy(
        fixture("changelog.txt"),
        tmp.path().join("data/changelog.txt"),
    )
    .expect("seed sibling doc");

    let root = tmp
        .path()
        .join("data")
        .canonicalize()
        .expect("canonicalize root");
    let found = discover(&root, SourceKind::Dir, &memories_dir);

    assert_eq!(
        rel_names(&found),
        vec!["changelog.txt".to_string()],
        "the memories/ subtree must be pruned entirely"
    );
}

#[test]
fn single_file_source_inside_memories_dir_is_ignored() {
    let (_tmp, memories_dir) = sandbox();
    let file = memories_dir.join("abc12345-a-managed-memory.md");
    fs::copy(fixture("guide.md"), &file).expect("seed managed memory file");
    let file = file.canonicalize().expect("canonicalize file");

    let found = discover(&file, SourceKind::File, &memories_dir);

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].classification, Classification::Ignored);
}

#[test]
fn single_file_source_outside_memories_dir_classifies_normally() {
    let (tmp, memories_dir) = sandbox();
    let file = tmp.path().join("standalone-guide.md");
    fs::copy(fixture("guide.md"), &file).expect("seed file");
    let file = file.canonicalize().expect("canonicalize file");

    let found = discover(&file, SourceKind::File, &memories_dir);

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].relative_path, Path::new("standalone-guide.md"));
    assert_eq!(
        found[0].classification,
        Classification::Document(DocumentFormat::Markdown)
    );
}

#[cfg(unix)]
#[test]
fn directory_symlink_is_not_followed() {
    let (tmp, memories_dir) = sandbox();
    let real_dir = tmp.path().join("real_docs");
    fs::create_dir_all(&real_dir).expect("mkdir real_docs");
    fs::copy(fixture("guide.md"), real_dir.join("guide.md")).expect("copy");

    let root = tmp.path().join("source_root");
    fs::create_dir_all(&root).expect("mkdir source_root");
    symlink(&real_dir, root.join("linked_dir")).expect("symlink dir");
    let root = root.canonicalize().expect("canonicalize root");

    let found = discover(&root, SourceKind::Dir, &memories_dir);
    assert!(
        found.is_empty(),
        "a directory symlink must not be followed: {found:?}"
    );
}

#[cfg(unix)]
#[test]
fn in_boundary_symlinked_file_is_accepted() {
    let (tmp, memories_dir) = sandbox();
    let root = tmp.path().join("source_root");
    fs::create_dir_all(&root).expect("mkdir source_root");
    let real_file = root.join("real_guide.md");
    fs::copy(fixture("guide.md"), &real_file).expect("copy real file");
    symlink(&real_file, root.join("linked_guide.md")).expect("symlink file");
    let root = root.canonicalize().expect("canonicalize root");

    let found = discover(&root, SourceKind::Dir, &memories_dir);
    let rels: BTreeSet<String> = rel_names(&found).into_iter().collect();

    assert_eq!(found.len(), 2);
    assert!(rels.contains("real_guide.md"));
    assert!(rels.contains("linked_guide.md"));
    let linked = found
        .iter()
        .find(|c| c.relative_path == Path::new("linked_guide.md"))
        .expect("linked entry present");
    assert_eq!(
        linked.classification,
        Classification::Document(DocumentFormat::Markdown)
    );
}

#[cfg(unix)]
#[test]
fn escaping_symlinked_file_is_rejected() {
    let (tmp, memories_dir) = sandbox();
    let outside_dir = TempDir::new().expect("outside tempdir");
    let outside_file = outside_dir.path().join("outside.md");
    fs::copy(fixture("guide.md"), &outside_file).expect("copy outside file");

    let root = tmp.path().join("source_root");
    fs::create_dir_all(&root).expect("mkdir source_root");
    symlink(&outside_file, root.join("escape.md")).expect("symlink outside");
    let root = root.canonicalize().expect("canonicalize root");

    let found = discover(&root, SourceKind::Dir, &memories_dir);
    assert!(
        found.is_empty(),
        "an out-of-boundary symlink target must be rejected: {found:?}"
    );
}
