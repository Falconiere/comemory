//! Real filesystem extraction, reinstallation, and conflict preservation.
use super::extract;

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
