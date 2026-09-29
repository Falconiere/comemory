#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/serve/routes/project_evidence.rs`: each query pair
//! reaches the evidence page request while the path alone names the
//! project, an unknown key or a non-numeric limit is the schema-edge `400`
//! naming it, and an attach body takes the path's project id over its own.

use comemory::domains::projects::evidence_add;
use comemory::errors::Error;
use comemory::serve::routes::project_evidence::{field, with_project_id};
use comemory::serve::routes::project_request::{body, query};
use comemory::utilities::error_code::{Class, classify};

fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
    v.iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

#[test]
fn query_pairs_become_an_evidence_page_whose_project_the_path_supplies() {
    let good = pairs(&[
        ("limit", "100"),
        ("cursor", "c"),
        ("kind", "commit"),
        ("trust", "pending"),
        ("workItemId", "w"),
        ("workspaceId", "ignored"),
    ]);
    let req = query(Ok::<_, ()>(good), field).unwrap();
    let got = (
        req.project_id.as_str(),
        req.limit,
        req.cursor.as_deref(),
        req.kind.as_deref(),
        req.trust.as_deref(),
        req.work_item_id.as_deref(),
    );
    assert_eq!(
        got,
        (
            "",
            Some(100),
            Some("c"),
            Some("commit"),
            Some("pending"),
            Some("w")
        )
    );
    for (key, bad) in [("limit", "ten"), ("projectId", "x"), ("order", "asc")] {
        let e = query(Ok::<_, ()>(pairs(&[(key, bad)])), field).unwrap_err();
        assert_eq!(classify(&e), ("invalid_request", Class::BadRequest), "{e}");
        let Error::Project(project) = &e else {
            panic!("{e:?}")
        };
        assert_eq!(
            serde_json::to_string(&project.details()).unwrap(),
            format!(r#"{{"field":"{key}","reason":"invalid"}}"#)
        );
    }
}

#[test]
fn an_attach_body_takes_the_paths_project_id() {
    let raw = br#"{"projectId":"other","idempotencyKey":"k","kind":"commit","source":"git"}"#;
    let req: evidence_add::Request = body(&with_project_id(raw, "from-path".into())).unwrap();
    assert_eq!(req.project_id, "from-path");
    for not_an_object in [&b"[1]"[..], b"nope"] {
        let e =
            body::<evidence_add::Request>(&with_project_id(not_an_object, "p".into())).unwrap_err();
        assert_eq!(e.to_string(), "body must be an object");
    }
    let e = body::<evidence_add::Request>(&with_project_id(br#"{"kind":"commit"}"#, "p".into()))
        .unwrap_err();
    assert_eq!(e.to_string(), "idempotencyKey is required");
}
