#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/show.rs`: a created project reads
//! back by id in either case; a malformed id is a `400`, an unknown one
//! `404 project_not_found`.

use comemory::config::{Config, Paths};
use comemory::domains::projects::authority::{self, Envelope};
use comemory::domains::projects::{create, show};
use comemory::store::connection;
use comemory::utilities::context::Ctx;
use comemory::utilities::error_code::{Class, classify};

#[test]
fn show_reads_back_refuses_malformed_and_answers_unknown_with_404() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    paths.ensure_dirs().unwrap();
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let req: create::Request = serde_json::from_value(serde_json::json!({
        "name": "Read me", "keyPrefix": "READ", "outcome": "Read back"
    }))
    .unwrap();
    let created = authority::run(&mut ctx, &Envelope::local_operator(), req)
        .unwrap()
        .project;

    let shown = authority::run(
        &mut ctx,
        &Envelope::local_operator(),
        show::Request {
            id: created.id.to_uppercase(),
        },
    )
    .unwrap();
    assert_eq!(shown.project, created);

    let e = authority::run(
        &mut ctx,
        &Envelope::local_operator(),
        show::Request {
            id: "not-a-uuid".into(),
        },
    )
    .unwrap_err();
    assert_eq!(classify(&e), ("invalid_request", Class::BadRequest));
    let unknown = "00000000-0000-4000-8000-000000000000";
    let e = authority::run(
        &mut ctx,
        &Envelope::local_operator(),
        show::Request { id: unknown.into() },
    )
    .unwrap_err();
    assert_eq!(classify(&e), ("project_not_found", Class::NotFound));
}
