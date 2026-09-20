//! Real filesystem extraction, reinstallation, conflict preservation, and
//! the per-install `.mcp.json` manifest.
use super::{extract, write_mcp_manifest};
use crate::prelude::Error;

#[test]
fn bundle_reinstall_preserves_identical_assets_and_rejects_user_edits() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("bundle");
    extract(&root).unwrap();
    extract(&root).unwrap();
    let skill = root.join("plugins/comemory/skills/agent-memory/SKILL.md");
    let bundled = std::fs::read_to_string(&skill).unwrap();
    assert_eq!(
        bundled,
        include_str!("../../../../../integrations/agent/skills/agent-memory/SKILL.md")
    );
    std::fs::write(&skill, "user's local edits").unwrap();
    assert!(extract(&root).is_err());
    assert_eq!(
        std::fs::read_to_string(&skill).unwrap(),
        "user's local edits"
    );
}

#[test]
fn bundle_ships_the_architecture_skill_with_its_whole_refresh_loop() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("bundle");
    extract(&root).unwrap();
    let skill = root.join("plugins/comemory/skills/architecture-map/SKILL.md");
    let bundled = std::fs::read_to_string(&skill).unwrap();
    assert_eq!(
        bundled,
        include_str!("../../../../../integrations/agent/skills/architecture-map/SKILL.md")
    );
    // An agent reading the skill must find every command of the loop, so it
    // never has to guess a flag.
    for command in [
        "comemory architecture scaffold",
        "comemory architecture save",
        "comemory architecture check",
        "comemory architecture show --repo <scope> --format mermaid",
    ] {
        assert!(bundled.contains(command), "skill omits {command:?}");
    }
}

#[cfg(unix)]
#[test]
fn bundle_refuses_symlink_destination() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target");
    std::fs::create_dir(&target).unwrap();
    let root = temp.path().join("bundle");
    std::os::unix::fs::symlink(&target, &root).unwrap();
    assert!(extract(&root).is_err());
    assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
}

#[test]
fn write_mcp_manifest_writes_the_command_and_args_for_a_fresh_bundle() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("bundle");
    extract(&root).unwrap();
    let plugin_root = root.join("plugins/comemory");
    let binary = temp.path().join("comemory-binary");
    std::fs::write(&binary, b"binary").unwrap();

    let manifest_path = write_mcp_manifest(&plugin_root, &binary).unwrap();

    assert_eq!(manifest_path, plugin_root.join(".mcp.json"));
    let written = std::fs::read_to_string(&manifest_path).unwrap();
    assert!(written.ends_with('\n'));
    let value: serde_json::Value = serde_json::from_str(&written).unwrap();
    assert_eq!(
        value["mcpServers"]["comemory"]["command"],
        binary.to_str().unwrap()
    );
    assert_eq!(
        value["mcpServers"]["comemory"]["args"],
        serde_json::json!(["mcp"])
    );
}

#[test]
fn write_mcp_manifest_rewrites_for_a_moved_binary_without_a_bundle_differs_refusal() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("bundle");
    extract(&root).unwrap();
    let plugin_root = root.join("plugins/comemory");
    let first = temp.path().join("comemory-before-move");
    let second = temp.path().join("comemory-after-move");
    std::fs::write(&first, b"one").unwrap();
    std::fs::write(&second, b"two").unwrap();
    write_mcp_manifest(&plugin_root, &first).unwrap();

    // Re-extracting the identical bundle and rewriting the manifest for a
    // different binary path must not trip the `bundle differs` refusal:
    // `.mcp.json` is deliberately outside the embedded-bytes equality check.
    extract(&root).unwrap();
    write_mcp_manifest(&plugin_root, &second).unwrap();

    let written = std::fs::read_to_string(plugin_root.join(".mcp.json")).unwrap();
    assert!(written.contains(second.to_str().unwrap()));
    assert!(!written.contains(first.to_str().unwrap()));
}

#[cfg(unix)]
#[test]
fn write_mcp_manifest_refuses_a_symlinked_target() {
    let temp = tempfile::tempdir().unwrap();
    let plugin_root = temp.path().join("plugins/comemory");
    std::fs::create_dir_all(&plugin_root).unwrap();
    let elsewhere = temp.path().join("elsewhere.json");
    std::fs::write(&elsewhere, "{}").unwrap();
    std::os::unix::fs::symlink(&elsewhere, plugin_root.join(".mcp.json")).unwrap();
    let binary = temp.path().join("comemory-binary");
    std::fs::write(&binary, b"binary").unwrap();

    let err = write_mcp_manifest(&plugin_root, &binary).unwrap_err();

    assert!(
        matches!(&err, Error::Usage(msg) if msg.starts_with("refusing symlink mcp manifest: ")
            && msg.ends_with(".mcp.json")),
        "unexpected error: {err}"
    );
    assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "{}");
}

/// The final `.mcp.json` has no symlink, but a symlink planted at the temp
/// path (`.mcp.json.tmp`) must still be refused rather than written
/// through: a naive `std::fs::write(&temp, ..)` follows a pre-existing
/// symlink and the subsequent rename would promote the attacker's target.
#[cfg(unix)]
#[test]
fn write_mcp_manifest_refuses_a_symlinked_temp_file() {
    let temp = tempfile::tempdir().unwrap();
    let plugin_root = temp.path().join("plugins/comemory");
    std::fs::create_dir_all(&plugin_root).unwrap();
    let elsewhere = temp.path().join("elsewhere.json");
    std::fs::write(&elsewhere, "{}").unwrap();
    std::os::unix::fs::symlink(&elsewhere, plugin_root.join(".mcp.json.tmp")).unwrap();
    let binary = temp.path().join("comemory-binary");
    std::fs::write(&binary, b"binary").unwrap();

    let err = write_mcp_manifest(&plugin_root, &binary).unwrap_err();

    assert!(
        matches!(&err, Error::Usage(msg) if msg.starts_with("refusing symlink mcp manifest temp file: ")
            && msg.ends_with(".mcp.json.tmp")),
        "unexpected error: {err}"
    );
    assert_eq!(
        std::fs::read_to_string(&elsewhere).unwrap(),
        "{}",
        "the symlink target outside the plugin root must be untouched"
    );
    assert!(
        !plugin_root.join(".mcp.json").exists(),
        "no manifest should have been promoted from the refused temp file"
    );
}
