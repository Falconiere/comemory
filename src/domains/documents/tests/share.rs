#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! A document's portable name, over real paths from a real checkout of this
//! repository's own `docs/` tree — including every way the name must be
//! refused rather than guessed.

use std::path::{Path, PathBuf};

use crate::domains::documents::share::{name_for, shared_id};
use crate::errors::Error;

/// The canonical repository every case shares under.
const REPO: &str = "Falconiere/comemory";

/// A temp checkout holding two real documents from this repository at a
/// nested path, so the source root and the repository root differ.
struct Checkout {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Checkout {
    /// Copy two real repository documents into `<tmp>/<name>/docs/guides/`.
    fn new(name: &str) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join(name);
        let guides = root.join("docs").join("guides");
        std::fs::create_dir_all(&guides).expect("create dirs");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/guides");
        for file in ["cloud-sync.md", "http-api.md"] {
            let body = std::fs::read_to_string(source.join(file))
                .unwrap_or_else(|e| panic!("read real {file}: {e}"));
            std::fs::write(guides.join(file), body).expect("write");
        }
        Self { _dir: dir, root }
    }

    /// The `docs/` directory, as a source root registered below the repo root.
    fn source_root(&self) -> PathBuf {
        self.root.join("docs")
    }
}

#[test]
fn the_same_document_from_two_checkouts_earns_one_name() {
    let first = Checkout::new("checkout-a");
    let second = Checkout::new("checkout-b-with-a-longer-name");
    assert_ne!(
        first.root, second.root,
        "the two checkouts must sit at different absolute paths"
    );

    let a = name_for(
        REPO,
        &first.root,
        &first.source_root(),
        "guides/cloud-sync.md",
    )
    .expect("name from the first checkout");
    let b = name_for(
        REPO,
        &second.root,
        &second.source_root(),
        "guides/cloud-sync.md",
    )
    .expect("name from the second checkout");

    assert_eq!(
        a, b,
        "the identity is the repo and the repo-relative path, nothing local"
    );
    assert_eq!(a.path, "docs/guides/cloud-sync.md");
    assert_eq!(a.shared_id.len(), 32);
    assert!(
        a.shared_id.chars().all(|c| c.is_ascii_hexdigit()),
        "{}",
        a.shared_id
    );
    // The decisive property: no absolute path survives into the name.
    for absolute in [
        first.root.to_str().expect("utf8"),
        second.root.to_str().expect("utf8"),
    ] {
        assert!(!a.path.contains(absolute), "{} leaked {absolute}", a.path);
        assert!(!a.shared_id.contains(absolute));
    }
}

#[test]
fn a_source_root_at_a_different_depth_earns_the_same_name() {
    let checkout = Checkout::new("checkout");

    // One machine registers `docs/`, another registers `docs/guides/`.
    let from_docs = name_for(
        REPO,
        &checkout.root,
        &checkout.source_root(),
        "guides/http-api.md",
    )
    .expect("registered at docs/");
    let from_guides = name_for(
        REPO,
        &checkout.root,
        &checkout.source_root().join("guides"),
        "http-api.md",
    )
    .expect("registered at docs/guides/");

    assert_eq!(
        from_docs, from_guides,
        "the name is relative to the repository, not to whatever depth was registered"
    );
    assert_eq!(from_docs.path, "docs/guides/http-api.md");
}

#[test]
fn a_path_that_escapes_the_repository_is_refused() {
    let checkout = Checkout::new("checkout");

    let refused = name_for(
        REPO,
        &checkout.root,
        &checkout.source_root(),
        "../../../etc/passwd",
    )
    .expect_err("a path outside the repository has no shared name");

    assert!(
        matches!(&refused, Error::BadRequest(m) if m.contains("outside the repository")),
        "got {refused:?}"
    );
}

#[test]
fn an_absolute_path_is_refused() {
    let checkout = Checkout::new("checkout");

    let refused = name_for(REPO, &checkout.root, &checkout.source_root(), "/etc/hosts")
        .expect_err("an absolute path is not repository-relative");

    assert!(
        matches!(&refused, Error::BadRequest(m) if m.contains("absolute")),
        "got {refused:?}"
    );
}

#[test]
fn a_path_resolving_to_the_repository_root_is_refused() {
    let checkout = Checkout::new("checkout");

    let refused = name_for(REPO, &checkout.root, &checkout.source_root(), "..")
        .expect_err("the repository root is not a document");

    assert!(matches!(refused, Error::BadRequest(_)), "got {refused:?}");
}

#[test]
fn an_empty_repository_is_refused() {
    let checkout = Checkout::new("checkout");

    for label in ["", "   "] {
        let refused = name_for(
            label,
            &checkout.root,
            &checkout.source_root(),
            "guides/cloud-sync.md",
        )
        .expect_err("a document with no canonical repository is withheld");
        assert!(
            matches!(&refused, Error::BadRequest(m) if m.contains("canonical repository")),
            "got {refused:?}"
        );
    }
}

#[test]
fn a_traversal_inside_the_repository_normalizes_rather_than_refusing() {
    let checkout = Checkout::new("checkout");

    // `docs/guides/../guides/cloud-sync.md` is the same document.
    let looped = name_for(
        REPO,
        &checkout.root,
        &checkout.source_root(),
        "guides/../guides/cloud-sync.md",
    )
    .expect("a traversal that stays inside resolves");
    let direct = name_for(
        REPO,
        &checkout.root,
        &checkout.source_root(),
        "guides/cloud-sync.md",
    )
    .expect("direct");

    assert_eq!(
        looped, direct,
        "two spellings of one path are one document, not two"
    );
}

#[test]
fn the_separator_stops_a_repo_and_path_from_colliding() {
    // Without the NUL, ("a/b", "c") and ("a", "b/c") would hash the same
    // bytes. The repo label cannot contain a NUL, so the split is unambiguous.
    assert_ne!(shared_id("a/b", "c"), shared_id("a", "b/c"));
    assert_eq!(shared_id(REPO, "docs/x.md"), shared_id(REPO, "docs/x.md"));
    assert_ne!(shared_id(REPO, "docs/x.md"), shared_id(REPO, "docs/X.md"));
}
