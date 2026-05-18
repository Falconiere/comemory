use qwick::memory::frontmatter::{Frontmatter, Kind};
use time::OffsetDateTime;

#[test]
fn round_trips_yaml() {
    let fm = Frontmatter {
        id: "a1b2c3d4".into(),
        kind: Kind::Decision,
        repo: "qwick-backend".into(),
        tags: vec!["postgres".into(), "migration".into()],
        author: "falconiere".into(),
        created: OffsetDateTime::from_unix_timestamp(1_734_700_000).unwrap(),
        quality: 4,
        schema: 1,
        content_hash: "a1b2c3d4e5f6".into(),
        references: Default::default(),
        relations: Default::default(),
    };
    let yaml = fm.to_yaml().unwrap();
    let back = Frontmatter::from_yaml(&yaml).unwrap();
    assert_eq!(back.id, fm.id);
    assert_eq!(back.kind, Kind::Decision);
    assert_eq!(back.tags, vec!["postgres".to_string(), "migration".into()]);
    assert_eq!(back.schema, 1);
}

#[test]
fn split_separates_frontmatter_and_body() {
    let raw = "---\nid: a1b2c3d4\nkind: note\nrepo: r\ntags: []\nauthor: a\ncreated: 2026-05-17T00:00:00Z\nquality: 3\nschema: 1\ncontent_hash: x\nreferences: {symbols: [], files: []}\nrelations: {supersedes: [], conflicts_with: [], derived_from: []}\n---\nhello body\n";
    let (fm, body) = Frontmatter::split(raw).unwrap();
    assert_eq!(fm.id, "a1b2c3d4");
    assert_eq!(body.trim(), "hello body");
}
