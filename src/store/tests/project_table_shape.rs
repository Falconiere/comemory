#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The carried-table view off the real declarations: which tables, in what
//! order, and the keys and references a bundle is checked against.

use comemory::store::project_table_shape::{Reference, carried};

#[test]
fn carried_lists_thirteen_tables_parents_first_without_receipts_or_bindings() {
    let names: Vec<String> = carried().into_iter().map(|s| s.name).collect();
    assert_eq!(names.len(), 13);
    assert_eq!(names[0], "projects", "the parent of every other row first");
    assert_eq!(
        names.last().map(String::as_str),
        Some("project_evidence_criteria")
    );
    for kept in ["project_command_receipts", "project_transfer_bindings"] {
        assert!(!names.iter().any(|n| n == kept), "{kept} never travels");
    }
}

#[test]
fn shapes_carry_keys_nullability_and_every_reference() {
    let shapes = carried();
    let find = |name: &str| shapes.iter().find(|s| s.name == name).expect(name);

    let dependencies = find("project_work_item_dependencies");
    assert_eq!(dependencies.key, ["project_id", "blocker_id", "blocked_id"]);

    let items = find("project_work_items");
    assert_eq!(items.key, ["id"]);
    let number = &items.columns[items.position("number").expect("number")];
    assert!(number.integer && number.not_null);
    let parent = &items.columns[items.position("parent_work_item_id").expect("parent")];
    assert!(!parent.integer && !parent.not_null);
    assert!(items.references.contains(&Reference {
        columns: vec!["parent_work_item_id".into(), "project_id".into()],
        table: "project_work_items".into(),
        target: vec!["id".into(), "project_id".into()],
    }));

    let criteria = find("project_evidence_criteria");
    assert!(
        criteria.columns.iter().all(|c| c.not_null),
        "key columns are not null"
    );
    assert_eq!(criteria.references.len(), 2);
    assert!(criteria.references.contains(&Reference {
        columns: vec!["criterion_id".into()],
        table: "project_criteria".into(),
        target: vec!["id".into()],
    }));
}
