#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/operation_rules.rs` (and the
//! work-item rules it shares): the twelve-kind fixture normalizes to its
//! `normalized` twin, the two proposal caps accept their limit and refuse
//! one past it with the platform's `cap_exceeded` details, and each field
//! rule refuses at its edge (`400` for a UUID or date, `422` for a length or
//! range) naming the field by its platform path.

use comemory::domains::projects::operation_rules::{
    OPERATIONS_BYTES_MAX, OPERATIONS_MAX, validate,
};
use comemory::domains::projects::operations::Operation;
use comemory::errors::Error;
use comemory::utilities::error_code::{Class, classify};
use serde_json::{Value, json};

fn fixture() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/projects/proposal_operations.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn ops(value: Value) -> Vec<Operation> {
    serde_json::from_value(value).unwrap()
}

/// `(class, details, message)` of the refusal `validate` gives `list`.
fn refusal(list: Value) -> (Class, String, String) {
    let e = validate(&mut ops(list)).expect_err("expected a refusal");
    let Error::Project(project) = &e else {
        panic!("not a project refusal: {e:?}")
    };
    let details = serde_json::to_string(&project.details()).unwrap();
    (classify(&e).1, details, e.to_string())
}

/// `count` work-item creates whose descriptions pad the serialized list to
/// exactly `bytes`.
fn padded(count: usize, bytes: usize) -> Vec<Operation> {
    let item = |n: usize, description: String| {
        json!({"op": "work_item.create", "workItem": {"kind": "task", "title": "t",
            "description": description, "id": format!("b0000000-0000-4000-8000-{n:012}")}})
    };
    let bare: Vec<Value> = (0..count).map(|n| item(n, String::new())).collect();
    let mut missing = bytes - serde_json::to_vec(&bare).unwrap().len();
    let list: Vec<Value> = (0..count)
        .map(|n| {
            let take = missing.min(8000);
            missing -= take;
            item(n, "x".repeat(take))
        })
        .collect();
    assert_eq!(missing, 0, "not enough room to pad to {bytes}");
    ops(Value::from(list))
}

#[test]
fn the_fixture_normalizes_uuids_and_dates() {
    let fixture = fixture();
    let mut list = ops(fixture["operations"].clone());
    validate(&mut list).unwrap();
    assert_eq!(serde_json::to_value(&list).unwrap(), fixture["normalized"]);
}

#[test]
fn an_empty_list_is_refused() {
    let (class, details, _) = refusal(json!([]));
    assert_eq!(class, Class::Unprocessable);
    assert_eq!(
        details,
        r#"{"field":"operations","reason":"too_short","limit":1}"#
    );
}

#[test]
fn the_operation_count_cap_admits_200_and_refuses_201_naming_200() {
    let op =
        json!({"op": "criterion.archive", "criterionId": "d0000000-0000-4000-8000-000000000001"});
    validate(&mut ops(Value::from(vec![op.clone(); OPERATIONS_MAX]))).unwrap();
    let (class, details, message) = refusal(Value::from(vec![op; OPERATIONS_MAX + 1]));
    assert_eq!(class, Class::Unprocessable);
    assert_eq!(
        details,
        r#"{"field":"operations","reason":"cap_exceeded","limit":200,"actual":201}"#
    );
    assert_eq!(message, "This proposal exceeds the operations limit of 200");
}

#[test]
fn the_byte_cap_admits_256_kib_and_refuses_one_byte_more() {
    validate(&mut padded(40, OPERATIONS_BYTES_MAX)).unwrap();
    let e = validate(&mut padded(40, OPERATIONS_BYTES_MAX + 1)).expect_err("over the cap");
    let Error::Project(project) = &e else {
        panic!("{e:?}")
    };
    assert_eq!(classify(&e).1, Class::Unprocessable);
    assert_eq!(
        serde_json::to_string(&project.details()).unwrap(),
        r#"{"field":"operations.bytes","reason":"cap_exceeded","limit":262144,"actual":262145}"#
    );
    assert_eq!(
        e.to_string(),
        "This proposal exceeds the operations.bytes limit of 262144"
    );
}

#[test]
fn each_field_rule_refuses_at_its_edge_naming_the_path() {
    let id = "d0000000-0000-4000-8000-000000000001";
    let cases = [
        (
            json!({"op": "criterion.archive", "criterionId": "nope"}),
            Class::BadRequest,
            r#"{"field":"operations.0.criterionId","reason":"invalid"}"#,
        ),
        (
            json!({"op": "milestone.create", "milestone": {"name": "n", "targetDate": "soon", "id": id}}),
            Class::BadRequest,
            r#"{"field":"operations.0.milestone.targetDate","reason":"invalid"}"#,
        ),
        (
            json!({"op": "work_item.create", "workItem": {"kind": "task", "title": "t".repeat(121), "id": id}}),
            Class::Unprocessable,
            r#"{"field":"operations.0.workItem.title","reason":"too_long","limit":120}"#,
        ),
        (
            json!({"op": "work_item.update", "workItemId": id, "patch": {"description": "x".repeat(8001)}}),
            Class::Unprocessable,
            r#"{"field":"operations.0.patch.description","reason":"too_long","limit":8000}"#,
        ),
        (
            json!({"op": "work_item.update", "workItemId": id, "patch": {"estimate": -1}}),
            Class::Unprocessable,
            r#"{"field":"operations.0.patch.estimate","reason":"too_small","limit":0}"#,
        ),
        (
            json!({"op": "work_item.update", "workItemId": id,
                "patch": {"assignee": {"principalType": "user", "principalId": ""}}}),
            Class::Unprocessable,
            r#"{"field":"operations.0.patch.assignee.principalId","reason":"too_short","limit":1}"#,
        ),
        (
            json!({"op": "criterion.create", "criterion": {"description": "x".repeat(501), "id": id}}),
            Class::Unprocessable,
            r#"{"field":"operations.0.criterion.description","reason":"too_long","limit":500}"#,
        ),
        (
            json!({"op": "milestone.update", "milestoneId": id, "patch": {"position": -1}}),
            Class::Unprocessable,
            r#"{"field":"operations.0.patch.position","reason":"too_small","limit":0}"#,
        ),
        (
            json!({"op": "project.update", "patch": {"constraints": vec!["c"; 51]}}),
            Class::Unprocessable,
            r#"{"field":"operations.0.patch.constraints","reason":"too_many","limit":50}"#,
        ),
        (
            json!({"op": "dependency.add", "blockerId": id, "blockedId": "B0000000"}),
            Class::BadRequest,
            r#"{"field":"operations.0.blockedId","reason":"invalid"}"#,
        ),
    ];
    for (op, class, details) in cases {
        let (got_class, got_details, _) = refusal(json!([op]));
        assert_eq!((got_class, got_details.as_str()), (class, details), "{op}");
    }
}
