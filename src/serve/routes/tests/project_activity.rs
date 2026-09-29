#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/serve/routes/project_activity.rs`: each query pair
//! reaches the activity request, the path alone names the project, and an
//! unknown key or a non-numeric limit is the schema-edge `400` naming it.

use comemory::errors::Error;
use comemory::serve::routes::project_activity::field;
use comemory::serve::routes::project_request::query;
use comemory::utilities::error_code::{Class, classify};

#[test]
fn query_pairs_become_an_activity_request_whose_id_the_path_supplies() {
    let pairs = |v: &[(&str, &str)]| -> Vec<(String, String)> {
        v.iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    };
    let good = pairs(&[
        ("limit", "200"),
        ("cursor", "c"),
        ("order", "asc"),
        ("workspaceId", "w"),
    ]);
    let req = query(Ok::<_, ()>(good), field).unwrap();
    let got = (
        req.id.as_str(),
        req.limit,
        req.cursor.as_deref(),
        req.order.as_deref(),
    );
    assert_eq!(got, ("", Some(200), Some("c"), Some("asc")));
    for (key, bad) in [
        ("limit", "ten"),
        ("id", "x"),
        ("projectId", "x"),
        ("after", "0"),
    ] {
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
