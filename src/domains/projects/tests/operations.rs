#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/operations.rs`: the twelve kinds
//! parse and re-serialize exactly (absent stays absent, `null` stays
//! `null`, zod key order kept), an identity key in any patch is refused, and
//! the bounded list refuses its 2,001st element on both the length-hinted
//! (`Value`) and the streaming (`&str`) path.

use comemory::domains::projects::operations::{OPERATIONS_SCHEMA_MAX, Operation, bounded};
use serde::Deserialize;
use serde_json::{Value, json};

/// The shared twelve-kind fixture.
fn fixture() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/projects/proposal_operations.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// A list behind the bounded deserializer, as a core request holds it.
#[derive(Deserialize, Debug)]
struct Holder {
    #[serde(deserialize_with = "bounded")]
    operations: Vec<Operation>,
}

fn parse(op: Value) -> Result<Operation, String> {
    serde_json::from_value(op).map_err(|e| e.to_string())
}

#[test]
fn every_kind_parses_and_serializes_back_to_its_input() {
    let submitted = fixture()["operations"].clone();
    let parsed: Vec<Operation> = serde_json::from_value(submitted.clone()).unwrap();
    let kinds: std::collections::BTreeSet<String> = submitted
        .as_array()
        .unwrap()
        .iter()
        .map(|op| op["op"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(kinds.len(), 12, "{kinds:?}");
    assert_eq!(serde_json::to_value(&parsed).unwrap(), submitted);
}

#[test]
fn a_patch_keeps_absent_and_null_apart() {
    let cleared = json!({"op": "work_item.update", "workItemId": "b0000000-0000-4000-8000-000000000002",
        "patch": {"milestoneId": null}});
    let untouched = json!({"op": "work_item.update", "workItemId": "b0000000-0000-4000-8000-000000000002",
        "patch": {}});
    for op in [cleared, untouched] {
        assert_eq!(
            serde_json::to_value(parse(op.clone()).unwrap()).unwrap(),
            op
        );
    }
}

#[test]
fn serialization_keeps_the_platform_key_order() {
    let op = &fixture()["operations"][2];
    let text = serde_json::to_string(&parse(op.clone()).unwrap()).unwrap();
    let keys: Vec<&str> = [
        "\"op\"",
        "parentWorkItemId",
        "milestoneId",
        "kind",
        "title",
        "description",
        "priority",
        "estimate",
        "assignee",
        "principalType",
        "principalId",
        "repo",
        "position",
        "\"id\"",
    ]
    .to_vec();
    let at: Vec<usize> = keys.iter().map(|k| text.find(k).unwrap()).collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "{text}");
}

#[test]
fn an_identity_key_in_any_patch_is_refused() {
    let id = "d0000000-0000-4000-8000-000000000001";
    for op in [
        json!({"op": "criterion.update", "criterionId": id, "patch": {"id": id}}),
        json!({"op": "criterion.update", "criterionId": id, "patch": {"workItemId": id}}),
        json!({"op": "milestone.update", "milestoneId": id, "patch": {"id": id}}),
        json!({"op": "work_item.update", "workItemId": id, "patch": {"id": id}}),
        json!({"op": "project.update", "patch": {"id": id}}),
    ] {
        let error = parse(op.clone()).expect_err(&op.to_string());
        assert!(error.contains("unknown field"), "{op}: {error}");
    }
}

#[test]
fn an_unknown_kind_enum_value_or_missing_field_is_refused() {
    let id = "d0000000-0000-4000-8000-000000000001";
    for op in [
        json!({"op": "work_item.cancel", "workItemId": id}),
        json!({"op": "work_item.create", "workItem": {"kind": "epic", "title": "t", "id": id}}),
        json!({"op": "milestone.create", "milestone": {"name": "n", "id": id}}),
        json!({"op": "dependency.add", "blockerId": id}),
        json!({"op": "criterion.archive", "criterionId": id, "extra": 1}),
    ] {
        assert!(parse(op.clone()).is_err(), "{op} parsed");
    }
}

/// zod's `.optional()` refuses `null`; only a `.nullable()` field takes it.
#[test]
fn null_on_a_field_that_is_not_nullable_is_refused() {
    let id = "d0000000-0000-4000-8000-000000000001";
    for op in [
        json!({"op": "milestone.update", "milestoneId": id, "patch": {"targetDate": null}}),
        json!({"op": "milestone.create", "milestone": {"name": "n", "targetDate": "2026-10-01",
            "description": null, "id": id}}),
        json!({"op": "work_item.update", "workItemId": id, "patch": {"title": null}}),
        json!({"op": "criterion.update", "criterionId": id, "patch": {"position": null}}),
        json!({"op": "project.update", "patch": {"name": null}}),
    ] {
        let error = parse(op.clone()).expect_err(&op.to_string());
        assert!(error.contains("invalid type: null"), "{op}: {error}");
    }
    let cleared = json!({"op": "project.update", "patch": {"targetDate": null}});
    assert_eq!(
        serde_json::to_value(parse(cleared.clone()).unwrap()).unwrap(),
        cleared
    );
}

#[test]
fn the_list_admits_2000_and_refuses_2001_on_both_paths() {
    let op =
        json!({"op": "criterion.archive", "criterionId": "d0000000-0000-4000-8000-000000000001"});
    for (count, admitted) in [
        (OPERATIONS_SCHEMA_MAX, true),
        (OPERATIONS_SCHEMA_MAX + 1, false),
    ] {
        let body = json!({"operations": vec![op.clone(); count]});
        let hinted = serde_json::from_value::<Holder>(body.clone());
        let streamed = serde_json::from_str::<Holder>(&body.to_string());
        for result in [hinted, streamed] {
            match result {
                Ok(holder) => assert!(admitted && holder.operations.len() == count),
                Err(e) => {
                    assert!(!admitted, "{count} refused: {e}");
                    assert!(e.to_string().contains("too many plan operations"), "{e}");
                }
            }
        }
    }
}
