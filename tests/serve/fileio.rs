//! File read/write for the editor: language detection, the blob-OID round
//! trip, the editable-extension allowlist, and `If-Match` optimistic
//! concurrency (stale match → conflict, not clobber).

use comemory::errors::Error;
use comemory::serve::fileio::{WriteOutcome, read_file, write_file};
use tempfile::TempDir;

#[test]
fn read_file_reports_lang_and_blob_oid() {
    let dir = TempDir::new().unwrap();
    let abs = dir.path().join("lib.rs");
    std::fs::write(&abs, "fn main() {}\n").unwrap();
    let view = read_file(&abs, "lib.rs").unwrap();
    assert_eq!(view.path, "lib.rs");
    assert_eq!(view.lang, "rust");
    assert_eq!(view.contents, "fn main() {}\n");
    assert_eq!(view.blob_oid.len(), 40, "git blob oid is 40 hex chars");
    // Matches what git would hash for the same bytes.
    let expected =
        git2::Oid::hash_object(git2::ObjectType::Blob, view.contents.as_bytes()).unwrap();
    assert_eq!(view.blob_oid, expected.to_string());
}

#[test]
fn read_file_rejects_non_utf8() {
    let dir = TempDir::new().unwrap();
    let abs = dir.path().join("bin.rs");
    std::fs::write(&abs, [0xff, 0xfe, 0x00]).unwrap();
    assert!(matches!(
        read_file(&abs, "bin.rs").unwrap_err(),
        Error::BadRequest(_)
    ));
}

#[test]
fn write_file_round_trip_updates_disk_and_oid() {
    let dir = TempDir::new().unwrap();
    let abs = dir.path().join("a.rs");
    std::fs::write(&abs, "fn a() {}\n").unwrap();
    let before = read_file(&abs, "a.rs").unwrap();

    let outcome = write_file(&abs, "fn a() { let x = 1; }\n", Some(&before.blob_oid)).unwrap();
    let new_oid = match outcome {
        WriteOutcome::Written { blob_oid } => blob_oid,
        WriteOutcome::Conflict { .. } => panic!("unexpected conflict"),
    };
    assert_ne!(new_oid, before.blob_oid);
    assert_eq!(
        std::fs::read_to_string(&abs).unwrap(),
        "fn a() { let x = 1; }\n"
    );
}

#[test]
fn write_file_stale_if_match_is_conflict_not_clobber() {
    let dir = TempDir::new().unwrap();
    let abs = dir.path().join("a.rs");
    std::fs::write(&abs, "v1\n").unwrap();
    let stale = read_file(&abs, "a.rs").unwrap().blob_oid;
    // Someone else edits the file on disk after our GET.
    std::fs::write(&abs, "v2-from-real-editor\n").unwrap();

    let outcome = write_file(&abs, "v3-from-web\n", Some(&stale)).unwrap();
    match outcome {
        WriteOutcome::Conflict { current_oid } => {
            let on_disk =
                git2::Oid::hash_object(git2::ObjectType::Blob, b"v2-from-real-editor\n").unwrap();
            assert_eq!(current_oid, on_disk.to_string());
        }
        WriteOutcome::Written { .. } => panic!("stale If-Match must conflict"),
    }
    // The on-disk content must be untouched.
    assert_eq!(
        std::fs::read_to_string(&abs).unwrap(),
        "v2-from-real-editor\n"
    );
}

#[test]
fn write_file_rejects_unsupported_extension() {
    let dir = TempDir::new().unwrap();
    let abs = dir.path().join("notes.txt");
    std::fs::write(&abs, "hello\n").unwrap();
    assert!(matches!(
        write_file(&abs, "bye\n", None).unwrap_err(),
        Error::Forbidden(_)
    ));
}
