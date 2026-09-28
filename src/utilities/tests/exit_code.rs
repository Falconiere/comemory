#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/utilities/exit_code.rs`: the per-variant rows moved
//! from `main.rs` keep their codes, and a project refusal's code follows its
//! class.

use comemory::errors::Error;
use comemory::utilities::error_code::Class;
use comemory::utilities::exit_code::{exit_code, exit_for_class};
use comemory::utilities::project_error::ProjectError;

/// One existing variant per sysexits bucket, pinned so the move out of
/// `main.rs` changed no code.
#[test]
fn existing_variants_keep_their_exit_codes() {
    let rows: Vec<(Error, i32)> = vec![
        (Error::NotFound("x".into()), 64),
        (Error::Usage("x".into()), 64),
        (Error::Unsupported("x".into()), 64),
        (Error::Frontmatter("x".into()), 65),
        (
            Error::IdCollision {
                id: "ab12cd34".into(),
            },
            65,
        ),
        (Error::Embedder("x".into()), 69),
        (Error::Forbidden("x".into()), 70),
        (Error::BadRequest("x".into()), 70),
        (Error::Other("x".into()), 70),
        (Error::Io(std::io::Error::other("x")), 74),
        (Error::Conflict("x".into()), 75),
        (Error::Busy("x".into()), 75),
        (Error::Config("x".into()), 78),
    ];
    for (err, expected) in &rows {
        assert_eq!(exit_code(err), *expected, "exit code for {err:?}");
    }
}

/// Every class has one exit code, and a project `forbidden` exits like the
/// crate's own `Error::Forbidden`.
#[test]
fn a_project_refusal_exits_by_its_class() {
    let table = [
        (Class::NotFound, 64),
        (Class::BadRequest, 64),
        (Class::NotImplemented, 64),
        (Class::Unprocessable, 65),
        (Class::Unavailable, 69),
        (Class::Unauthorized, 70),
        (Class::Forbidden, 70),
        (Class::Internal, 70),
        (Class::Conflict, 75),
        (Class::Locked, 75),
    ];
    for (class, expected) in table {
        assert_eq!(exit_for_class(class), expected, "exit for {class:?}");
    }
    assert_eq!(
        exit_code(&Error::Project(ProjectError::ExecutionActorForbidden)),
        exit_code(&Error::Forbidden("x".into())),
    );
}
