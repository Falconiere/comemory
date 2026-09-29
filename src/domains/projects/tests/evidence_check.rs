#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/evidence_check.rs`: every cap
//! accepts its limit and refuses one past it with `422 {field, reason,
//! limit}`, every unparsable value answers `400` naming its field, a
//! malformed `repo` or `commitSha` is `422 invalid_format`, and accepted
//! values come back canonical.

use comemory::domains::projects::evidence_add::Request;
use comemory::domains::projects::evidence_check::validate;
use comemory::errors::Error;
use comemory::utilities::error_code::{Class, classify};
use serde_json::{Value, json};

const PROJECT: &str = "11111111-1111-4111-8111-111111111111";

/// An `external_url` attach with `extra` merged over it.
fn request(extra: Value) -> Request {
    let mut body = json!({
        "projectId": PROJECT, "idempotencyKey": "k", "kind": "external_url", "source": "ci"
    });
    body.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    serde_json::from_value(body).unwrap()
}

/// The class, `field` and `reason` (and `limit`, when set) of a refusal.
fn refusal(extra: Value) -> (Class, Value) {
    let e: Error = match validate(request(extra)) {
        Ok(_) => panic!("expected a refusal"),
        Err(e) => e,
    };
    let Error::Project(project) = &e else {
        panic!("not a project refusal: {e:?}");
    };
    (
        classify(&e).1,
        serde_json::to_value(project.details()).unwrap(),
    )
}

fn hex(n: usize) -> String {
    "a".repeat(n)
}

#[test]
fn caps_accept_their_limit_and_refuse_one_past_it() {
    let at = [
        json!({"source": "s".repeat(120)}),
        json!({"externalId": "x".repeat(256)}),
        json!({"url": format!("https://example.com/{}", "p".repeat(2048 - 20))}),
        json!({"commitSha": hex(256)}),
        json!({"criterionIds": vec!["c0000000-0000-4000-8000-000000000001"; 20]}),
    ];
    for extra in at {
        validate(request(extra.clone())).unwrap_or_else(|e| panic!("{extra}: {e}"));
    }
    let over = [
        (
            json!({"source": "s".repeat(121)}),
            "source",
            "too_long",
            120,
        ),
        (json!({"source": ""}), "source", "too_short", 1),
        (
            json!({"externalId": "x".repeat(257)}),
            "externalId",
            "too_long",
            256,
        ),
        (json!({"externalId": ""}), "externalId", "too_short", 1),
        (
            json!({"url": format!("https://example.com/{}", "p".repeat(2048 - 19))}),
            "url",
            "too_long",
            2048,
        ),
        (json!({"commitSha": hex(257)}), "commitSha", "too_long", 256),
        (
            json!({"criterionIds": vec!["c0000000-0000-4000-8000-000000000001"; 21]}),
            "criterionIds",
            "too_many",
            20,
        ),
    ];
    for (extra, field, reason, limit) in over {
        assert_eq!(
            refusal(extra.clone()),
            (
                Class::Unprocessable,
                json!({"field": field, "reason": reason, "limit": limit})
            ),
            "{extra}"
        );
    }
}

#[test]
fn unparsable_values_answer_400_and_malformed_claims_422() {
    let bad_request = [
        (json!({"projectId": "not-a-uuid"}), "projectId"),
        (json!({"workItemId": "nope"}), "workItemId"),
        (json!({"kind": "screenshot"}), "kind"),
        (json!({"url": "not a url"}), "url"),
        (json!({"url": " https://ci.example/7"}), "url"),
        (json!({"criterionIds": [PROJECT, "x"]}), "criterionIds.1"),
    ];
    for (extra, field) in bad_request {
        assert_eq!(
            refusal(extra.clone()),
            (
                Class::BadRequest,
                json!({"field": field, "reason": "invalid"})
            ),
            "{extra}"
        );
    }
    let malformed = [
        (json!({"repo": "not-a-repo"}), "repo"),
        (json!({"repo": "https://github.com/a/b/c"}), "repo"),
        (json!({"commitSha": "xyz"}), "commitSha"),
        (json!({"commitSha": "abc 123"}), "commitSha"),
    ];
    for (extra, field) in malformed {
        assert_eq!(
            refusal(extra.clone()),
            (
                Class::Unprocessable,
                json!({"field": field, "reason": "invalid_format"})
            ),
            "{extra}"
        );
    }
}

#[test]
fn accepted_values_come_back_canonical() {
    let valid = validate(request(json!({
        "projectId": PROJECT.to_uppercase(),
        "workItemId": "B0000000-0000-4000-8000-000000000002",
        "repo": "  Falconiere/Comemory ",
        "commitSha": "ABCDEF0123",
        "criterionIds": ["C0000000-0000-4000-8000-000000000001"],
        "metadata": {"run": 7}
    })))
    .unwrap();
    assert_eq!(valid.project_id, PROJECT);
    assert_eq!(valid.raw_project_id, PROJECT.to_uppercase());
    assert_eq!(
        valid.work_item_id.as_deref(),
        Some("b0000000-0000-4000-8000-000000000002")
    );
    assert_eq!(valid.repo.as_deref(), Some("falconiere/comemory"));
    assert_eq!(valid.commit_sha.as_deref(), Some("abcdef0123"));
    assert_eq!(
        valid.criterion_ids,
        ["c0000000-0000-4000-8000-000000000001"]
    );
    assert_eq!(Value::Object(valid.metadata), json!({"run": 7}));
}
