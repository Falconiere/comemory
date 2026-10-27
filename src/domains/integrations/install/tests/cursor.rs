//! Cursor file install against a fixture bundle staged from the real skills
//! tree, plus `mcp.json` merge rules.
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

fn skill_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in std::fs::read_dir(&current).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[test]
fn install_copies_real_skills_and_merges_mcp_server() {
    let task = tempfile::tempdir().unwrap();
    let root = stage_bundle(task.path());
    let config = task.path().join("cursor");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("mcp.json"),
        r#"{"mcpServers": {"foreign": {"command": "/bin/other", "args": []}}}"#,
    )
    .unwrap();

    install(&config, &root).unwrap();

    for file in skill_files(std::path::Path::new(SKILLS)) {
        let relative = file.strip_prefix(SKILLS).unwrap();
        let mirrored = config.join("skills-cursor").join(relative);
        assert_eq!(
            std::fs::read(&mirrored).unwrap(),
            std::fs::read(&file).unwrap(),
            "skill file must arrive byte-identical: {}",
            relative.display()
        );
    }
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(config.join("mcp.json")).unwrap()).unwrap();
    assert_eq!(
        value["mcpServers"]["foreign"]["command"], "/bin/other",
        "foreign servers survive the merge"
    );
    assert_eq!(
        value["mcpServers"]["comemory"]["args"],
        serde_json::json!(["mcp"])
    );
    assert!(
        value["mcpServers"]["comemory"]["command"]
            .as_str()
            .is_some_and(|command| !command.is_empty()),
        "the server command must name a binary"
    );

    let before = std::fs::read(config.join("mcp.json")).unwrap();
    install(&config, &root).unwrap();
    assert_eq!(
        std::fs::read(config.join("mcp.json")).unwrap(),
        before,
        "rerun over identical content is byte-identical"
    );
}

#[test]
fn install_refreshes_a_stale_comemory_command() {
    let task = tempfile::tempdir().unwrap();
    let root = stage_bundle(task.path());
    let config = task.path().join("cursor");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("mcp.json"),
        r#"{"mcpServers": {"comemory": {"command": "/old/binary", "args": ["mcp"]}}}"#,
    )
    .unwrap();

    install(&config, &root).unwrap();

    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(config.join("mcp.json")).unwrap()).unwrap();
    assert_ne!(
        value["mcpServers"]["comemory"]["command"], "/old/binary",
        "a stale command must be refreshed to this binary"
    );
}

#[test]
fn install_refuses_unparsable_mcp_json_without_touching_it() {
    let task = tempfile::tempdir().unwrap();
    let root = stage_bundle(task.path());
    let config = task.path().join("cursor");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("mcp.json"), "{not json").unwrap();

    let err = install(&config, &root).unwrap_err().to_string();

    assert!(err.contains("unparsable"), "unexpected error: {err}");
    assert_eq!(
        std::fs::read_to_string(config.join("mcp.json")).unwrap(),
        "{not json"
    );
}

#[test]
fn install_refuses_edited_skill_content() {
    let task = tempfile::tempdir().unwrap();
    let root = stage_bundle(task.path());
    let config = task.path().join("cursor");
    install(&config, &root).unwrap();
    let first = skill_files(&config.join("skills-cursor"));
    assert!(!first.is_empty());
    std::fs::write(&first[0], "operator edits").unwrap();

    let err = install(&config, &root).unwrap_err().to_string();

    assert!(err.contains("differs"), "unexpected error: {err}");
}
