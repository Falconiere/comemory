#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/domains/documents/source/discover.rs`.

use std::collections::BTreeSet;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use comemory::config::paths::Paths;
use comemory::domains::documents::document::DocumentFormat;
use comemory::domains::documents::source::SourceKind;
use comemory::domains::documents::source::classify::Classification;
use comemory::domains::documents::source::discover::discover;
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

fn rel_names(found: &[comemory::domains::documents::source::discover::Candidate]) -> Vec<String> {
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

    let mut found = discover(&root, SourceKind::Dir, &memories_dir).candidates;
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

    let found = discover(&root, SourceKind::Dir, &memories_dir).candidates;
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

    let found = discover(&root, SourceKind::Dir, &memories_dir).candidates;
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

    let found = discover(&root, SourceKind::Dir, &memories_dir).candidates;
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
    let found = discover(&root, SourceKind::Dir, &memories_dir).candidates;

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

    let found = discover(&file, SourceKind::File, &memories_dir).candidates;

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].classification, Classification::Ignored);
}

#[test]
fn single_file_source_outside_memories_dir_classifies_normally() {
    let (tmp, memories_dir) = sandbox();
    let file = tmp.path().join("standalone-guide.md");
    fs::copy(fixture("guide.md"), &file).expect("seed file");
    let file = file.canonicalize().expect("canonicalize file");

    let found = discover(&file, SourceKind::File, &memories_dir).candidates;

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

    let found = discover(&root, SourceKind::Dir, &memories_dir).candidates;
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

    let found = discover(&root, SourceKind::Dir, &memories_dir).candidates;
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

    let found = discover(&root, SourceKind::Dir, &memories_dir).candidates;
    assert!(
        found.is_empty(),
        "an out-of-boundary symlink target must be rejected: {found:?}"
    );
}

// ---------------------------------------------------------------------------
// AC-15: a walk that could not read everything says so, because its absences
// are about to decide what gets tombstoned on every peer.
// ---------------------------------------------------------------------------

/// A docs tree with one real fixture at the root and one in a subdirectory.
#[cfg(unix)]
fn tree_with_subdirectory(tmp: &TempDir) -> PathBuf {
    let root = tmp.path().join("docs");
    let sub = root.join("deep");
    fs::create_dir_all(&sub).expect("mkdir");
    fs::copy(fixture("guide.md"), root.join("guide.md")).expect("copy root fixture");
    fs::copy(fixture("changelog.txt"), sub.join("changelog.txt")).expect("copy sub fixture");
    root.canonicalize().expect("canonicalize root")
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("set permissions");
}

#[cfg(unix)]
#[test]
fn a_fully_readable_walk_reports_itself_complete() {
    let (tmp, memories_dir) = sandbox();
    let root = tree_with_subdirectory(&tmp);

    let found = discover(&root, SourceKind::Dir, &memories_dir);

    assert!(
        found.complete,
        "nothing was denied, so an incomplete verdict below would prove nothing"
    );
    assert_eq!(found.candidates.len(), 2, "{:?}", found.candidates);
}

#[cfg(unix)]
#[test]
fn an_unreadable_subdirectory_makes_the_walk_incomplete() {
    let (tmp, memories_dir) = sandbox();
    let root = tree_with_subdirectory(&tmp);
    set_mode(&root.join("deep"), 0o000);

    let found = discover(&root, SourceKind::Dir, &memories_dir);

    // Restore before any assertion can unwind with the directory unreadable,
    // or the TempDir cannot clean itself up.
    set_mode(&root.join("deep"), 0o755);

    assert!(
        !found.complete,
        "an entry was skipped, so absences are not evidence of deletion"
    );
    assert_eq!(
        found
            .candidates
            .iter()
            .map(|c| c.relative_path.to_string_lossy().to_string())
            .collect::<Vec<_>>(),
        vec!["guide.md".to_string()],
        "the readable half is still indexed — a denied subdirectory must not \
         stop the rest of a source"
    );
}

#[cfg(unix)]
#[test]
fn permission_denied_on_the_root_itself_is_incomplete_and_empty() {
    let (tmp, memories_dir) = sandbox();
    let root = tree_with_subdirectory(&tmp);
    set_mode(&root, 0o000);

    let found = discover(&root, SourceKind::Dir, &memories_dir);

    set_mode(&root, 0o755);

    assert!(!found.complete, "the root itself could not be read");
    assert!(
        found.candidates.is_empty(),
        "and nothing was found, which is exactly the list that must NOT be \
         treated as authoritative: {:?}",
        found.candidates
    );
}
