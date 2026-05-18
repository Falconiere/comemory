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
