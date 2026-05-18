use qwick_memory::serve::dto::edge_id;

#[test]
fn edge_id_is_deterministic() {
    let a = edge_id("m:a1b2c3d4", "InRepo", "r:qwick-backend");
    let b = edge_id("m:a1b2c3d4", "InRepo", "r:qwick-backend");
    assert_eq!(a, b);
    assert!(a.starts_with("e:"));
    assert_eq!(a.len(), 18, "format is e:<16-hex>");
}

#[test]
fn edge_id_changes_with_kind() {
    let a = edge_id("m:a1b2c3d4", "InRepo", "r:qwick-backend");
    let b = edge_id("m:a1b2c3d4", "Tagged", "r:qwick-backend");
    assert_ne!(a, b);
}

#[test]
fn edge_id_changes_with_endpoints() {
    let a = edge_id("m:aaaa", "InRepo", "r:one");
    let b = edge_id("m:bbbb", "InRepo", "r:one");
    assert_ne!(a, b);
}

use qwick_memory::serve::dto::{
    EdgeDto, EdgeRef, GraphPayload, NodeDetail, NodeDto, SearchResponse, SearchResult,
};
use serde_json::json;

#[test]
fn node_dto_roundtrip() {
    let n = NodeDto {
        id: "m:a1b2c3d4".into(),
        label: "a1b2c3d4".into(),
        kind: "Memory".into(),
        props: json!({ "quality": 4, "created": "2026-05-17T14:30:00Z" }),
    };
    let s = serde_json::to_string(&n).unwrap();
    let back: NodeDto = serde_json::from_str(&s).unwrap();
    assert_eq!(n, back);
}

#[test]
fn graph_payload_roundtrip() {
    let p = GraphPayload {
        nodes: vec![NodeDto {
            id: "r:one".into(),
            label: "one".into(),
            kind: "Repo".into(),
            props: json!({}),
        }],
        edges: vec![EdgeDto {
            id: "e:0123456789abcdef".into(),
            source: "m:aaaa".into(),
            target: "r:one".into(),
            kind: "InRepo".into(),
            props: json!({}),
        }],
    };
    let s = serde_json::to_string(&p).unwrap();
    let back: GraphPayload = serde_json::from_str(&s).unwrap();
    assert_eq!(p, back);
}

#[test]
fn search_response_roundtrip() {
    let r = SearchResponse {
        results: vec![SearchResult {
            id: "m:aaaa".into(),
            label: "aaaa".into(),
            kind: "Memory".into(),
        }],
    };
    let s = serde_json::to_string(&r).unwrap();
    let back: SearchResponse = serde_json::from_str(&s).unwrap();
    assert_eq!(r, back);
}

#[test]
fn node_detail_roundtrip_memory() {
    let d = NodeDetail {
        node: NodeDto {
            id: "m:aaaa".into(),
            label: "aaaa".into(),
            kind: "Memory".into(),
            props: json!({ "quality": 3 }),
        },
        memory_body: Some("# body".into()),
        frontmatter: Some(json!({ "id": "aaaa" })),
        outbound: vec![EdgeRef {
            edge_kind: "InRepo".into(),
            target: Some("r:one".into()),
            source: None,
        }],
        inbound: vec![EdgeRef {
            edge_kind: "ReferencesFile".into(),
            source: Some("m:bbbb".into()),
            target: None,
        }],
    };
    let s = serde_json::to_string(&d).unwrap();
    let back: NodeDetail = serde_json::from_str(&s).unwrap();
    assert_eq!(d, back);
}

#[test]
fn node_detail_omits_body_for_non_memory() {
    let d = NodeDetail {
        node: NodeDto {
            id: "r:one".into(),
            label: "one".into(),
            kind: "Repo".into(),
            props: json!({}),
        },
        memory_body: None,
        frontmatter: None,
        outbound: vec![],
        inbound: vec![],
    };
    let s = serde_json::to_string(&d).unwrap();
    assert!(!s.contains("memory_body"));
    assert!(!s.contains("frontmatter"));
}
