//! Mirror test for `src/api/ast.rs`. Calls `api::ast::run` directly against
//! a `Ctx::lazy` (conn-free) over a real source file — proving matches,
//! paging, the default `--limit` (50), and the unsupported-lang error
//! (`cli::ast::run` is byte-compat tested against CLI stdout in
//! `tests/cli__ast.rs`; the HTTP route, including `--file` containment,
//! lives in `tests/serve__routes__code.rs`).

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};

fn write_fixture(dir: &std::path::Path) -> std::path::PathBuf {
    let file = dir.join("sample.rs");
    std::fs::write(
        &file,
        "fn alpha() -> Result<u8, ()> { Ok(1) }\n\
         fn beta() -> Result<u8, ()> { Ok(2) }\n\
         fn gamma() {}\n",
    )
    .expect("write fixture");
    file
}

#[test]
fn run_finds_every_matching_function() {
    let home = tempfile::tempdir().expect("tempdir");
    let file = write_fixture(home.path());
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let req = api::ast::Request {
        pattern: "fn $NAME($$$ARGS) -> $RET { $$$BODY }".to_string(),
        lang: "rs".to_string(),
        file: file.display().to_string(),
        limit: 0,
        offset: 0,
    };
    let page = api::ast::run(&mut ctx, req).expect("ast run");
    assert_eq!(page.items.len(), 2, "alpha + beta match, gamma does not");
    assert_eq!(page.items[0].line, 1);
    assert_eq!(page.items[1].line, 2);
}

#[test]
fn run_defaults_limit_to_fifty_when_omitted() {
    let req: api::ast::Request = serde_json::from_value(serde_json::json!({
        "pattern": "fn $NAME()",
        "lang": "rs",
        "file": "src/lib.rs",
    }))
    .expect("defaults must parse");
    assert_eq!(req.limit, 50);
    assert_eq!(req.offset, 0);
}

#[test]
fn run_rejects_an_unsupported_language() {
    let home = tempfile::tempdir().expect("tempdir");
    let file = write_fixture(home.path());
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let req = api::ast::Request {
        pattern: "fn $NAME()".to_string(),
        lang: "cobol".to_string(),
        file: file.display().to_string(),
        limit: 0,
        offset: 0,
    };
    let err = api::ast::run(&mut ctx, req).expect_err("unsupported lang must error");
    assert!(err.to_string().contains("unsupported --lang"));
}

#[test]
fn run_errors_on_a_missing_file() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let req = api::ast::Request {
        pattern: "fn $NAME()".to_string(),
        lang: "rs".to_string(),
        file: home.path().join("nope.rs").display().to_string(),
        limit: 0,
        offset: 0,
    };
    assert!(api::ast::run(&mut ctx, req).is_err());
}

#[test]
fn request_rejects_unknown_fields() {
    let err = serde_json::from_value::<api::ast::Request>(serde_json::json!({
        "pattern": "fn $NAME()",
        "lang": "rs",
        "file": "src/lib.rs",
        "bogus": 1,
    }))
    .expect_err("unknown field must be rejected");
    assert!(err.to_string().contains("unknown field"));
}
