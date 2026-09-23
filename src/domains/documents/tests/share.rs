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

// ---------------------------------------------------------------------------
// AC-10: the secret scan refuses a whole revision, and says which rule did it.
// The marker is built at runtime from two halves: a literal matching the
// shipped `aws-access-key-id` pattern would trip this repository's own
// secret-content guardrail on this file, and loosening that gate to fixture a
// test would be the wrong trade. It is only ever written into a temp copy.
// ---------------------------------------------------------------------------

/// A value the shipped rule set really matches, assembled so the pattern
/// never appears whole in this source file.
fn marker() -> String {
    format!("{}{}", "AKIA", "QWERTYUIOPASDFGH")
}

/// A payload over the real extracted chunks of a real repository document.
fn revision_of(path: &str) -> crate::domains::documents::replica_payload::DocumentRevisionV1 {
    use crate::domains::documents::document::{DocumentFormat, extract};
    use crate::domains::documents::replica_payload::{ChunkWire, DocumentRevisionV1, LinkWire};

    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    let body = std::fs::read(&file).unwrap_or_else(|e| panic!("read real {path}: {e}"));
    let extracted =
        extract::extract(DocumentFormat::Markdown, &body, "doc").expect("real extraction");
    DocumentRevisionV1 {
        shared_id: shared_id(REPO, path),
        repo: REPO.to_string(),
        path: path.to_string(),
        title: extracted.title.clone(),
        format: "markdown".to_string(),
        revision_hash: "d".repeat(64),
        chunks: extracted
            .chunks
            .iter()
            .map(|c| ChunkWire {
                ordinal: c.ordinal as i64,
                heading_path: c.heading_path.join(" > "),
                char_start: c.char_range.0 as i64,
                char_end: c.char_range.1 as i64,
                line_start: c.line_range.0 as i64,
                line_end: c.line_range.1 as i64,
                simhash: c.simhash as i64,
                text: c.text.clone(),
            })
            .collect(),
        links: vec![LinkWire {
            ordinal: 0,
            target: "docs/guides/http-api.md".to_string(),
        }],
    }
}

#[test]
fn blocked_a_clean_real_document_is_shareable() {
    let clean = revision_of("docs/guides/cloud-sync.md");

    assert_eq!(
        crate::domains::documents::share::blocked_reason(&clean),
        None,
        "a real repository document carries no credential, so the gate must \
         pass it — otherwise the refusals below prove nothing"
    );
    assert!(!clean.chunks.is_empty(), "and it really has passages");
}

#[test]
fn blocked_a_marker_in_a_passage_refuses_the_whole_revision() {
    let mut revision = revision_of("docs/guides/cloud-sync.md");
    let last = revision.chunks.len() - 1;
    revision.chunks[last].text.push_str(&marker());

    assert_eq!(
        crate::domains::documents::share::blocked_reason(&revision).as_deref(),
        Some("aws-access-key-id"),
        "one passage is enough; the revision is shared whole or not at all"
    );
}

#[test]
fn blocked_a_marker_in_a_heading_or_title_refuses_it_too() {
    let mut in_heading = revision_of("docs/guides/cloud-sync.md");
    in_heading.chunks[0].heading_path = format!("Setup > {}", marker());
    assert_eq!(
        crate::domains::documents::share::blocked_reason(&in_heading).as_deref(),
        Some("aws-access-key-id"),
        "a heading leaves the machine as surely as a passage"
    );

    let mut in_title = revision_of("docs/guides/cloud-sync.md");
    in_title.title = format!("Cloud sync {}", marker());
    assert_eq!(
        crate::domains::documents::share::blocked_reason(&in_title).as_deref(),
        Some("aws-access-key-id")
    );
}

#[test]
fn blocked_a_marker_in_a_link_target_refuses_it_too() {
    let mut revision = revision_of("docs/guides/cloud-sync.md");
    revision.links[0].target = format!("docs/{}.md", marker());

    assert_eq!(
        crate::domains::documents::share::blocked_reason(&revision).as_deref(),
        Some("aws-access-key-id"),
        "a link target is replicated metadata and is scanned with the rest"
    );
}
