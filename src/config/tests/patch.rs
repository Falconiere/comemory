#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Coverage for `src/config/patch.rs` against real files in a temp dir: the
//! missing-file bootstrap, the preserve-other-keys round trip, the atomic
//! tmp cleanup, and the not-a-table refusal.

use comemory::config::patch::{patch_config_file, section};
use toml::Value;

#[test]
fn creates_the_file_when_absent_and_preserves_unrelated_keys_afterwards() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    patch_config_file(&path, |root| {
        section(root, "reinforce")?.insert("enabled".into(), Value::Boolean(false));
        Ok(())
    })
    .unwrap();
    let first: Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(first["reinforce"]["enabled"], Value::Boolean(false));
    assert!(
        !path.with_extension("toml.tmp").exists(),
        "tmp file cleaned up"
    );

    patch_config_file(&path, |root| {
        section(root, "retrieval")?.insert("rrf_k".into(), Value::Float(30.0));
        Ok(())
    })
    .unwrap();
    let second: Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        second["reinforce"]["enabled"],
        Value::Boolean(false),
        "kept"
    );
    assert_eq!(second["retrieval"]["rrf_k"], Value::Float(30.0));
}

#[test]
fn refuses_a_section_key_that_is_not_a_table_and_leaves_the_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "retrieval = 5\n").unwrap();
    let err = patch_config_file(&path, |root| {
        section(root, "retrieval")?.insert("rrf_k".into(), Value::Float(30.0));
        Ok(())
    })
    .unwrap_err();
    assert!(
        err.to_string().contains("[retrieval] is not a table"),
        "{err}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "retrieval = 5\n");
}

#[test]
fn a_patch_error_leaves_the_file_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[rank]\ndecay = 0.5\n").unwrap();
    let err = patch_config_file(&path, |_root| {
        Err(comemory::errors::Error::Config("nope".into()))
    })
    .unwrap_err();
    assert!(err.to_string().contains("nope"));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[rank]\ndecay = 0.5\n"
    );
}
