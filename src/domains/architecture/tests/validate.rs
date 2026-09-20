#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

use std::path::{Path, PathBuf};

use comemory::domains::architecture::model::{MAX_BYTES, Model};
use comemory::domains::architecture::validate::validate;

/// Every `.rs` path under the real `src/` tree of this repository, relative to
/// the crate root — the same shape `store::indexed_files::list_for_repo`
/// returns, taken from the actual files rather than a stand-in list.
fn indexed_paths() -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    let mut stack = vec![root.join("src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let rel = path.strip_prefix(root).unwrap();
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    assert!(
        out.len() > 100,
        "expected a real src tree, got {}",
        out.len()
    );
    out
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/common/fixtures/architecture")
        .join(name)
}

fn parse(name: &str) -> serde_json::Result<Model> {
    serde_json::from_str(&std::fs::read_to_string(fixture(name)).unwrap())
}

fn rejection(name: &str) -> String {
    let model = parse(name).unwrap_or_else(|e| panic!("{name} should parse: {e}"));
    let err = validate(&model, "comemory", &indexed_paths())
        .expect_err(&format!("{name} must be rejected"));
    err.to_string()
}

#[test]
fn a_model_whose_members_cover_indexed_files_validates() {
    let model = parse("valid.json").unwrap();
    validate(&model, "comemory", &indexed_paths()).expect("valid fixture must pass");
}

#[test]
fn every_structural_violation_is_rejected_by_name() {
    for (fixture, needle) in [
        ("duplicate_id.json", "duplicate component id domains_graph"),
        ("unknown_group.json", "undeclared group nope"),
        (
            "dangling_edge.json",
            "\"ghost\" is not a declared component",
        ),
        ("bad_id.json", "\"1bad\""),
        ("self_edge.json", "to itself"),
        (
            "missing_member.json",
            "\"src/does-not-exist\" matches no indexed file",
        ),
        ("external_with_members.json", "must own no members"),
        ("wrong_repo.json", "\"somewhere-else\""),
        ("bad_schema.json", "unsupported architecture schema 7"),
        ("long_summary.json", "281-character summary"),
    ] {
        let message = rejection(fixture);
        assert!(
            message.contains(needle),
            "{fixture}: {message:?} does not name {needle:?}"
        );
    }
}

#[test]
fn an_unknown_enum_variant_or_field_fails_to_parse() {
    for name in ["bad_kind.json", "unknown_field.json"] {
        assert!(parse(name).is_err(), "{name} must not deserialize");
    }
}

#[test]
fn a_model_past_the_byte_ceiling_is_rejected() {
    let mut model = parse("valid.json").unwrap();
    let filler = model.components[1].clone();
    let mut n = 0;
    while serde_json::to_vec(&model).unwrap().len() <= MAX_BYTES {
        let mut c = filler.clone();
        c.id = format!("pad_{n}");
        c.summary = "x".repeat(200);
        model.components.push(c);
        n += 1;
    }
    let err = validate(&model, "comemory", &indexed_paths()).expect_err("must be rejected");
    assert!(
        err.to_string().contains("at most 32768 bytes allowed"),
        "{err}"
    );
}

#[test]
fn a_member_prefix_must_stop_at_a_path_boundary() {
    let mut model = parse("valid.json").unwrap();
    // `src/cl` is a textual prefix of `src/cli/...` but not a directory of it.
    model.components[1].members = vec!["src/cl".into()];
    let err = validate(&model, "comemory", &indexed_paths()).expect_err("must be rejected");
    assert!(err.to_string().contains("matches no indexed file"), "{err}");
}
