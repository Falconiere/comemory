//! Hermes skills install against a fixture bundle staged from the real
//! skills tree.
use super::install;

const SKILLS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/integrations/agent/skills");

/// Stage `<task>/bundle/plugins/comemory/skills` from the real skills tree
/// so assertions compare against genuine bundle content, not fixtures.
fn stage_bundle(task: &std::path::Path) -> std::path::PathBuf {
    let root = task.join("bundle");
    let dst = root.join("plugins/comemory/skills");
    std::fs::create_dir_all(&dst).unwrap();
    super::super::dirs::mirror_dir(std::path::Path::new(SKILLS), &dst).unwrap();
    root
}

#[test]
fn install_copies_real_skills_byte_identical_and_reruns_clean() {
    let task = tempfile::tempdir().unwrap();
    let root = stage_bundle(task.path());
    let config = task.path().join("hermes");

    install(&config, &root).unwrap();

    let mut names: Vec<_> = std::fs::read_dir(std::path::Path::new(SKILLS))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    assert!(!names.is_empty());
    for name in &names {
        let mirrored = config.join("skills").join(name);
        assert!(
            mirrored.is_dir(),
            "skill dir must arrive under its own name: {}",
            mirrored.display()
        );
    }
    let before = std::fs::read(config.join("skills").join(&names[0]).join("SKILL.md")).unwrap();
    install(&config, &root).unwrap();
    assert_eq!(
        std::fs::read(config.join("skills").join(&names[0]).join("SKILL.md")).unwrap(),
        before,
        "rerun over identical content is byte-identical"
    );
}

#[test]
fn install_refuses_edited_skill_content() {
    let task = tempfile::tempdir().unwrap();
    let root = stage_bundle(task.path());
    let config = task.path().join("hermes");
    install(&config, &root).unwrap();
    let skill = config.join("skills").join("agent-memory").join("SKILL.md");
    std::fs::write(&skill, "operator edits").unwrap();

    let err = install(&config, &root).unwrap_err().to_string();

    assert!(err.contains("differs"), "unexpected error: {err}");
    assert_eq!(std::fs::read(&skill).unwrap(), b"operator edits");
}
