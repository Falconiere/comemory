#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Real HTTP jobs use the same selected-folder lifecycle and containment.
#[path = "common/serve_bin.rs"]
mod serve_bin;
use serde_json::json;
use serve_bin::ServeHome;
use tempfile::tempdir;

#[test]
fn folder_job_reconciles_and_refuses_outside_roots() {
    let folder = tempdir().unwrap();
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/docs/getting-started.md"),
        folder.path().join("guide.md"),
    )
    .unwrap();
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/src/utilities/digest.rs"),
        folder.path().join("digest.rs"),
    )
    .unwrap();
    let server = ServeHome::with_args_env(
        &["--allow-path", folder.path().to_str().unwrap()],
        &[
            ("COMEMORY_SYNC_DAEMON", "0"),
            ("COMEMORY_DOCUMENT_EMBED_CMD", ""),
            ("COMEMORY_CODE_EMBED_CMD", ""),
        ],
    );
    let request = json!({"path":folder.path(), "repo":"folder-test"});
    let first = server.job("/run/embedding", &request);
    assert_eq!(first["documents"]["sources"][0]["indexed"], 1);
    assert_eq!(first["code"]["files_indexed"], 1);
    std::fs::remove_file(folder.path().join("guide.md")).unwrap();
    let second = server.job("/run/embedding", &request);
    assert_eq!(second["documents"]["sources"][0]["removed"], 1);
    assert_eq!(second["code"]["files_indexed"], 0);
    let outside = tempdir().unwrap();
    assert_eq!(
        server
            .post_raw("/run/embedding", &json!({"path":outside.path()}))
            .0,
        403
    );
    let read_only = ServeHome::with_args(&["--read-only"]);
    assert_eq!(read_only.post_raw("/run/embedding", &request).0, 405);
}

#[test]
#[ignore = "requires a compatible real local provider in COMEMORY_TEST_EMBED_CMD"]
fn real_document_inference_works_through_http_jobs_and_find() {
    let command = std::env::var("COMEMORY_TEST_EMBED_CMD").expect("configure real local provider");
    let folder = tempdir().unwrap();
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/docs/guides/byo-vectors.md"),
        folder.path().join("guide.md"),
    )
    .unwrap();
    let server = ServeHome::with_args_env(
        &["--allow-path", folder.path().to_str().unwrap()],
        &[
            ("COMEMORY_SYNC_DAEMON", "0"),
            ("COMEMORY_DOCUMENT_EMBED_CMD", command.as_str()),
            ("COMEMORY_CODE_EMBED_CMD", ""),
        ],
    );
    let result = server.job("/run/embedding", &json!({"path":folder.path()}));
    assert!(
        result["documents"]["sources"][0]["embedded"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(result["documents"]["sources"][0]["embedding_failed"], 0);
    let found = server.post(
        "/find",
        &json!({"query":"How can I supply vectors for semantic retrieval?", "domain":"document"}),
    );
    assert!(
        found["hits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hit| hit["score_parts"]["ann"].is_number()),
        "ANN evidence missing: {found}"
    );
    std::fs::remove_file(folder.path().join("guide.md")).unwrap();
    server.job("/run/embedding", &json!({"path":folder.path()}));
    let found = server.post(
        "/find",
        &json!({"query":"How can I supply vectors for semantic retrieval?", "domain":"document"}),
    );
    assert!(found["hits"].as_array().unwrap().is_empty());
}
