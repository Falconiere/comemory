use comemory::retrieval::bundle;
use comemory::store::connection;
use tempfile::tempdir;

#[test]
fn assemble_returns_empty_bundle_when_no_ids() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    let bundle = bundle::assemble(&conn, "advisory lock", &[]).expect("assemble");
    assert_eq!(bundle.query, "advisory lock");
    assert!(bundle.memories.is_empty());
    assert!(bundle.code_refs.is_empty());
    assert!(bundle.relations.is_empty());
}

#[test]
fn assemble_pulls_memory_rows_by_id() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES('m1','s','decision','h','Use Postgres for analytics','t','t','x.md')",
        [],
    )
    .expect("seed");

    let bundle = bundle::assemble(&conn, "postgres", &["m1".to_string()]).expect("assemble");
    assert_eq!(bundle.memories.len(), 1);
    assert_eq!(bundle.memories[0].id, "m1");
    assert_eq!(bundle.memories[0].kind, "decision");
    assert_eq!(bundle.memories[0].body, "Use Postgres for analytics");

    let v: serde_json::Value = serde_json::to_value(&bundle).expect("json");
    assert_eq!(v["query"], "postgres");
    assert_eq!(v["memories"][0]["id"], "m1");
}
