#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/domains/projects/view.rs` over a real store: an
//! unreadable stored JSON list reads as `[]`, and a stored timestamp outside
//! the representable range is an `internal_error`, never an empty date.

use comemory::config::{Config, Paths};
use comemory::domains::projects::authority::{self, Envelope};
use comemory::domains::projects::{create, show};
use comemory::store::connection;
use comemory::utilities::context::Ctx;
use comemory::utilities::error_code::{Class, classify};

#[test]
fn corrupt_stored_values_degrade_or_refuse_as_the_platform_would() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    paths.ensure_dirs().unwrap();
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let id = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let req: create::Request = serde_json::from_value(serde_json::json!({
            "name": "Corrupt", "keyPrefix": "BAD", "outcome": "o", "constraints": ["kept?"]
        }))
        .unwrap();
        authority::run(&mut ctx, &Envelope::local_operator(), req)
            .unwrap()
            .project
            .id
    };

    conn.execute_batch("UPDATE projects SET constraints = 'not json', non_goals = '{}'")
        .unwrap();
    let project = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        authority::run(
            &mut ctx,
            &Envelope::local_operator(),
            show::Request { id: id.clone() },
        )
        .unwrap()
        .project
    };
    assert_eq!(project.constraints, Vec::<String>::new());
    assert_eq!(project.non_goals, Vec::<String>::new());

    conn.execute_batch(&format!("UPDATE projects SET created_at = {}", i64::MAX))
        .unwrap();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let e =
        authority::run(&mut ctx, &Envelope::local_operator(), show::Request { id }).unwrap_err();
    assert_eq!(classify(&e), ("internal_error", Class::Internal));
    assert!(e.to_string().contains("projects.created_at"), "{e}");
}
