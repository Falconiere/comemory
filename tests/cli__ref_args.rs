//! Integration tests for `comemory::cli::ref_args::collect`. Exercised against
//! the REAL comemory git repo (`CARGO_MANIFEST_DIR` is the crate root, a git
//! checkout with committed files) so anchor capture hits the same on-disk
//! layout a user repo would have. No mocks.

use std::path::{Path, PathBuf};

use comemory::cli::ref_args::collect;
use comemory::git_utils::blob_oid_at_head;

/// The real comemory checkout this test crate lives in.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn ref_symbol_qualifies_repo_and_captures_committed_anchor() {
    let root = repo_root();
    let (refs, warnings) = collect(
        &[],
        &["src/cli/save.rs:run".to_string()],
        "comemory",
        Some(&root),
    )
    .expect("collect");

    assert!(refs.files.is_empty(), "no file refs expected");
    assert_eq!(refs.symbols.len(), 1, "one symbol ref");
    let r = &refs.symbols[0];
    assert_eq!(
        r.id, "comemory:src/cli/save.rs:run",
        "unprefixed value qualifies to the passed repo"
    );
    let expected_blob = blob_oid_at_head(&root, "src/cli/save.rs")
        .expect("blob lookup")
        .expect("src/cli/save.rs is tracked");
    assert_eq!(
        r.blob.as_deref(),
        Some(expected_blob.as_str()),
        "blob anchor must equal blob_oid_at_head for the file"
    );
    assert!(r.commit.is_some(), "tracked ref captures the HEAD commit");
    assert!(
        warnings.is_empty(),
        "a tracked ref emits no warning, got {warnings:?}"
    );
}

#[test]
fn explicit_repo_prefix_is_honored_in_dst_id() {
    let root = repo_root();
    let (refs, _) = collect(
        &["other:lib/x.rs".to_string()],
        &[],
        "comemory",
        Some(&root),
    )
    .expect("collect");
    assert_eq!(refs.files.len(), 1);
    assert_eq!(
        refs.files[0].id, "other:lib/x.rs",
        "leading `repo:` segment overrides the save repo"
    );
}

#[test]
fn untracked_path_is_unpinned_with_warning() {
    let root = repo_root();
    let (refs, warnings) = collect(
        &["src/does/not/exist/zzz.nope".to_string()],
        &[],
        "comemory",
        Some(&root),
    )
    .expect("collect");
    assert_eq!(refs.files.len(), 1);
    let r = &refs.files[0];
    assert_eq!(r.blob, None, "untracked path captures no blob");
    assert_eq!(r.commit, None, "untracked path captures no commit");
    assert_eq!(
        warnings.len(),
        1,
        "an unpinned ref emits exactly one advisory warning"
    );
    assert!(
        warnings[0].contains("src/does/not/exist/zzz.nope") || warnings[0].contains("untracked"),
        "warning should describe the unpinned ref, got {:?}",
        warnings[0]
    );
}

#[test]
fn cross_repo_ref_is_unpinned_with_warning() {
    let root = repo_root();
    // `other:` != the save repo, so the ref cannot be resolved against the
    // local git tree and must save unpinned.
    let (refs, warnings) = collect(
        &["other:src/cli/save.rs".to_string()],
        &[],
        "comemory",
        Some(&root),
    )
    .expect("collect");
    assert_eq!(refs.files[0].blob, None, "cross-repo ref is unpinned");
    assert_eq!(warnings.len(), 1, "cross-repo ref warns");
}

#[test]
fn ref_symbol_without_symbol_segment_is_usage_error() {
    let root = repo_root();
    let err = collect(&[], &["foo.rs".to_string()], "comemory", Some(&root))
        .expect_err("a symbol value with no `:symbol` must be a usage error");
    let msg = format!("{err}");
    assert!(
        msg.contains("foo.rs"),
        "usage error must name the offending value, got {msg:?}"
    );
}

#[test]
fn empty_ref_value_is_usage_error() {
    let root = repo_root();
    // A whole value of "::" has empty segments — malformed.
    let err = collect(&["::".to_string()], &[], "comemory", Some(&root))
        .expect_err("an empty-segment value must be a usage error");
    assert!(format!("{err}").contains("malformed"), "got {err}");
}

#[test]
fn comma_split_yields_multiple_refs() {
    let root = repo_root();
    let (refs, _) = collect(
        &["src/lib.rs,src/main.rs".to_string()],
        &[],
        "comemory",
        Some(&root),
    )
    .expect("collect");
    assert_eq!(refs.files.len(), 2, "one comma-joined flag yields two refs");
    let ids: Vec<&str> = refs.files.iter().map(|r| r.id.as_str()).collect();
    assert!(ids.contains(&"comemory:src/lib.rs"), "got {ids:?}");
    assert!(ids.contains(&"comemory:src/main.rs"), "got {ids:?}");
}

#[test]
fn path_is_normalized_repo_root_relative_from_a_subdir() {
    // The test process cwd is the crate root (CARGO_MANIFEST_DIR). Passing the
    // crate's PARENT as repo_root makes the cwd a *subdir* of the repo root, so
    // a cwd-relative ref must be rewritten to its repo-root-relative form
    // (`<cratedir>/Cargo.toml`) rather than left as the bare `Cargo.toml`.
    let crate_dir = repo_root();
    let parent = crate_dir.parent().expect("crate has a parent dir");
    let crate_name = crate_dir
        .file_name()
        .and_then(|s| s.to_str())
        .expect("crate dir name");
    let (refs, _) =
        collect(&["Cargo.toml".to_string()], &[], "comemory", Some(parent)).expect("collect");
    let expected = format!("comemory:{crate_name}/Cargo.toml");
    assert_eq!(
        refs.files[0].id, expected,
        "a ref given from a subdir must normalize to the repo-root-relative dst_id"
    );
}

#[test]
fn no_repo_root_leaves_path_verbatim_and_unpinned() {
    let (refs, warnings) = collect(
        &["src/cli/save.rs".to_string()],
        &[],
        "comemory",
        None::<&Path>,
    )
    .expect("collect");
    assert_eq!(
        refs.files[0].id, "comemory:src/cli/save.rs",
        "without a repo root the path is used verbatim"
    );
    assert_eq!(refs.files[0].blob, None, "no repo root → unpinned");
    assert_eq!(warnings.len(), 1, "no repo root warns");
}
