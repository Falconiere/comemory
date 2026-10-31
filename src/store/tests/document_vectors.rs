#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Document vector identity, revision guards, search and cascade.

use comemory::domains::documents::document::DocumentFormat;
use comemory::domains::documents::document::writer::UpdateOutcome;
use comemory::store::{document_vectors, documents};
use tempfile::TempDir;

use crate::test_common::document_writer_support::{self as support, *};

#[test]
fn vector_is_scoped_to_provider_and_removed_on_chunk_replacement() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = support::open_db(&tmp);
    let path = write_fixture(&tmp, "notes.txt", b"original document text");
    let UpdateOutcome::Indexed { document_id } =
        index(&mut conn, &path, "notes.txt", DocumentFormat::Txt)
    else {
        panic!("indexed")
    };
    let mut pending = document_vectors::pending(&conn, SOURCE_ID, "model-a").expect("pending");
    let chunk = pending.remove(0);
    let mut vector = vec![0.0; document_vectors::DIM];
    vector[0] = 1.0;
    assert!(document_vectors::publish(&conn, &chunk, "model-a", &vector).expect("publish"));
    assert_eq!(
        document_vectors::search(&conn, "model-a", &vector, 5).expect("search")[0].0,
        document_id
    );
    assert!(
        document_vectors::search(&conn, "model-b", &vector, 5)
            .expect("search")
            .is_empty()
    );
    std::fs::write(&path, b"new document content").expect("edit");
    index(&mut conn, &path, "notes.txt", DocumentFormat::Txt);
    assert!(
        document_vectors::search(&conn, "model-a", &vector, 5)
            .expect("search")
            .is_empty()
    );
    assert!(!document_vectors::publish(&conn, &chunk, "model-a", &vector).expect("guard"));
    assert!(
        documents::get_document(&conn, &document_id)
            .expect("read")
            .is_some()
    );
}

#[test]
fn older_provider_generation_cannot_overwrite_newer_vector() {
    let tmp = TempDir::new().expect("tempdir");
    let mut conn = support::open_db(&tmp);
    let path = write_fixture(&tmp, "notes.txt", b"document text");
    index(&mut conn, &path, "notes.txt", DocumentFormat::Txt);
    let chunk = document_vectors::pending(&conn, SOURCE_ID, "old")
        .expect("pending")
        .remove(0);
    let old = document_vectors::begin(&conn, SOURCE_ID, "old").expect("begin");
    let new = document_vectors::begin(&conn, SOURCE_ID, "new").expect("begin");
    let mut vector = vec![0.0; document_vectors::DIM];
    vector[0] = 1.0;
    assert!(
        document_vectors::publish_guarded(&conn, &chunk, "new", &vector, &new).expect("publish")
    );
    assert!(
        !document_vectors::publish_guarded(&conn, &chunk, "old", &vector, &old).expect("guard")
    );
    assert_eq!(
        document_vectors::search(&conn, "new", &vector, 1)
            .expect("search")
            .len(),
        1
    );
}
