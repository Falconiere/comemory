#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The SessionEnd contract: the installed command and the payload it returns.
//!
//! Every case writes only into a `tempfile::tempdir()` — a colocated test has
//! no isolated `HOME`, so it must never reach the real `~/.claude`.

use std::path::PathBuf;

use comemory::domains::capture::hook::{
    DEFAULT_SETTINGS_PATH, HOOK_MARKER, install, session_end_target,
};

#[test]
fn install_writes_session_end_command() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let report = install(&path, false).expect("install");
    assert!(report.command.contains(HOOK_MARKER));
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("SessionEnd"));
    assert!(raw.contains(HOOK_MARKER));
    // Idempotent refresh
    install(&path, false).expect("reinstall");
    let raw2 = std::fs::read_to_string(&path).unwrap();
    assert_eq!(raw2.matches(HOOK_MARKER).count(), 1);
}

#[test]
fn the_installed_command_names_the_default_settings_file_claude_code_reads() {
    assert_eq!(DEFAULT_SETTINGS_PATH, ".claude/settings.json");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(DEFAULT_SETTINGS_PATH);
    let report = install(&path, false).expect("install into the default relative path");
    assert_eq!(report.path, path, "install reports the file it wrote");
    assert!(
        path.exists(),
        "install creates the parent directory the default path implies"
    );
}

#[test]
fn a_foreign_session_end_block_is_refused_without_force_and_kept_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(
        &path,
        r#"{"hooks":{"SessionEnd":[{"hooks":[{"type":"command","command":"make tidy"}]}]}}"#,
    )
    .unwrap();

    let err = install(&path, false).expect_err("a hand-written block must not be clobbered");
    assert!(err.to_string().contains("--force"), "{err}");
    let untouched = std::fs::read_to_string(&path).unwrap();
    assert!(untouched.contains("make tidy"));
    assert!(
        !untouched.contains(HOOK_MARKER),
        "the refusal wrote nothing"
    );

    install(&path, true).expect("force merges beside the foreign entry");
    let merged = std::fs::read_to_string(&path).unwrap();
    assert!(merged.contains("make tidy"), "the foreign hook survives");
    assert_eq!(merged.matches(HOOK_MARKER).count(), 1);
}

#[test]
fn session_end_target_takes_the_transcript_path_and_session_id_the_tool_sends() {
    let raw = r#"{"session_id":"8e9f54e3-a984-46ab-8403-135ee920cbca",
        "transcript_path":"/Users/dev/.claude/projects/p/8e9f54e3.jsonl",
        "hook_event_name":"SessionEnd","reason":"clear"}"#;
    let (path, id) = session_end_target(raw).expect("a real SessionEnd payload decodes");
    assert_eq!(
        path,
        Some(PathBuf::from(
            "/Users/dev/.claude/projects/p/8e9f54e3.jsonl"
        ))
    );
    assert_eq!(id.as_deref(), Some("8e9f54e3-a984-46ab-8403-135ee920cbca"));
}

#[test]
fn session_end_target_accepts_either_field_alone_and_ignores_surrounding_whitespace() {
    let (path, id) = session_end_target("  {\"session_id\":\"abc\"}\n  ").expect("id only");
    assert_eq!(path, None);
    assert_eq!(id.as_deref(), Some("abc"));

    let (path, id) =
        session_end_target(r#"{"transcript_path":"/tmp/s.jsonl"}"#).expect("path only");
    assert_eq!(path, Some(PathBuf::from("/tmp/s.jsonl")));
    assert_eq!(id, None);
}

#[test]
fn session_end_target_treats_empty_strings_as_absent() {
    // Claude Code writes "" rather than omitting a field it has no value for.
    let err = session_end_target(r#"{"session_id":"","transcript_path":""}"#)
        .expect_err("two empty strings name no session");
    assert!(
        err.to_string()
            .contains("missing session_id and transcript_path"),
        "{err}"
    );
}

#[test]
fn session_end_target_rejects_a_payload_that_is_not_session_end_json() {
    let err = session_end_target("not json at all").expect_err("malformed stdin");
    assert!(
        err.to_string()
            .contains("--from-hook expects SessionEnd JSON on stdin"),
        "{err}"
    );

    let err = session_end_target("{}").expect_err("an empty object names no session");
    assert!(
        err.to_string()
            .contains("missing session_id and transcript_path"),
        "{err}"
    );
}
