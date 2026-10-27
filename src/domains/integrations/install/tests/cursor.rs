//! Cursor file install against the real embedded bundle: skill links into
//! `skills/`, plus the `mcp.json` merge rules — ownership of only the
//! `comemory` key, file order and foreign bytes kept, permissions kept, and
//! no rewrite when already current.
use std::path::{Path, PathBuf};

use super::install;

/// Extract the real embedded bundle under `task`.
fn extract_bundle(task: &Path) -> PathBuf {
    let root = task.join("integrations").join("1.0.0");
    super::super::bundle::extract(&root).unwrap();
    root
}

fn server() -> serde_json::Value {
    serde_json::json!({"command": "/opt/comemory/bin/comemory", "args": ["mcp"]})
}

#[test]
fn install_links_skills_into_the_personal_dir_and_merges_the_server() {
    let task = tempfile::tempdir().unwrap();
    let root = extract_bundle(task.path());
    let config = task.path().join("cursor");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("mcp.json"),
        r#"{"mcpServers": {"foreign": {"command": "/bin/other", "args": []}}}"#,
    )
    .unwrap();

    install(&config, &root, &server()).unwrap();

    assert!(
        config.join("skills/agent-memory/SKILL.md").is_file(),
        "skills land in Cursor's personal skills dir"
    );
    assert!(
        !config.join("skills-cursor").exists(),
        "skills-cursor/ is reserved for Cursor's built-ins"
    );
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(config.join("mcp.json")).unwrap()).unwrap();
    assert_eq!(value["mcpServers"]["foreign"]["command"], "/bin/other");
    assert_eq!(value["mcpServers"]["comemory"], server());

    let before = std::fs::read(config.join("mcp.json")).unwrap();
    let modified = std::fs::metadata(config.join("mcp.json"))
        .unwrap()
        .modified()
        .unwrap();
    install(&config, &root, &server()).unwrap();
    assert_eq!(std::fs::read(config.join("mcp.json")).unwrap(), before);
    assert_eq!(
        std::fs::metadata(config.join("mcp.json"))
            .unwrap()
            .modified()
            .unwrap(),
        modified,
        "an already-current entry leaves the file untouched"
    );
}

#[test]
fn the_merge_keeps_key_order_and_foreign_bytes() {
    let task = tempfile::tempdir().unwrap();
    let root = extract_bundle(task.path());
    let config = task.path().join("cursor");
    std::fs::create_dir_all(&config).unwrap();
    let foreign = r#"{"command":"/bin/zeta","env":{"Z_TOKEN":"z","A_TOKEN":"a"}}"#;
    std::fs::write(
        config.join("mcp.json"),
        format!(r#"{{"zz": 1, "mcpServers": {{"zeta": {foreign}, "alpha": {{"command": "/bin/alpha"}}}}, "aa": 2}}"#),
    )
    .unwrap();

    install(&config, &root, &server()).unwrap();

    let body = std::fs::read_to_string(config.join("mcp.json")).unwrap();
    assert!(
        body.contains(foreign),
        "foreign server bytes survive: {body}"
    );
    let position = |needle: &str| body.find(needle).unwrap();
    assert!(position("\"zz\"") < position("\"mcpServers\""));
    assert!(position("\"mcpServers\"") < position("\"aa\""));
    assert!(position("\"zeta\"") < position("\"alpha\""));
    assert!(position("\"alpha\"") < position("\"comemory\""));
}

#[cfg(unix)]
#[test]
fn the_merge_keeps_the_files_permissions() {
    use std::os::unix::fs::PermissionsExt as _;

    let task = tempfile::tempdir().unwrap();
    let root = extract_bundle(task.path());
    let config = task.path().join("cursor");
    std::fs::create_dir_all(&config).unwrap();
    let path = config.join("mcp.json");
    std::fs::write(
        &path,
        r#"{"mcpServers": {"secret": {"env": {"TOKEN": "x"}}}}"#,
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

    install(&config, &root, &server()).unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "a token-bearing config stays private");
}

#[test]
fn install_refreshes_a_stale_comemory_command() {
    let task = tempfile::tempdir().unwrap();
    let root = extract_bundle(task.path());
    let config = task.path().join("cursor");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("mcp.json"),
        r#"{"mcpServers": {"comemory": {"command": "/old/binary", "args": ["mcp"]}}}"#,
    )
    .unwrap();

    install(&config, &root, &server()).unwrap();

    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(config.join("mcp.json")).unwrap()).unwrap();
    assert_eq!(value["mcpServers"]["comemory"], server());
}

#[test]
fn install_refuses_unparsable_or_non_object_config_without_touching_it() {
    let task = tempfile::tempdir().unwrap();
    let root = extract_bundle(task.path());
    let config = task.path().join("cursor");
    std::fs::create_dir_all(&config).unwrap();
    for body in ["{not json", r#"{"mcpServers": []}"#] {
        std::fs::write(config.join("mcp.json"), body).unwrap();

        let err = install(&config, &root, &server()).unwrap_err().to_string();

        assert!(err.contains("not a JSON object"), "unexpected error: {err}");
        assert_eq!(
            std::fs::read_to_string(config.join("mcp.json")).unwrap(),
            body
        );
    }
}

#[cfg(unix)]
#[test]
fn a_symlinked_config_is_merged_into_its_target_and_stays_a_link() {
    let task = tempfile::tempdir().unwrap();
    let root = extract_bundle(task.path());
    let config = task.path().join("cursor");
    let dotfiles = task.path().join("dotfiles");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::create_dir_all(&dotfiles).unwrap();
    std::fs::write(dotfiles.join("mcp.json"), r#"{"mcpServers": {}}"#).unwrap();
    std::os::unix::fs::symlink(dotfiles.join("mcp.json"), config.join("mcp.json")).unwrap();

    install(&config, &root, &server()).unwrap();

    assert!(
        config
            .join("mcp.json")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink(),
        "a dotfile manager's link survives the merge"
    );
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dotfiles.join("mcp.json")).unwrap()).unwrap();
    assert_eq!(value["mcpServers"]["comemory"], server());
}

#[test]
fn an_empty_config_counts_as_an_empty_object() {
    let task = tempfile::tempdir().unwrap();
    let root = extract_bundle(task.path());
    let config = task.path().join("cursor");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("mcp.json"), "\n").unwrap();

    install(&config, &root, &server()).unwrap();

    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(config.join("mcp.json")).unwrap()).unwrap();
    assert_eq!(value["mcpServers"]["comemory"], server());
}

#[test]
fn an_unmergeable_config_is_refused_before_any_skill_is_linked() {
    let task = tempfile::tempdir().unwrap();
    let root = extract_bundle(task.path());
    let config = task.path().join("cursor");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("mcp.json"), "{not json").unwrap();

    install(&config, &root, &server()).unwrap_err();

    assert!(
        !config.join("skills").exists(),
        "a refused merge leaves no half-installed skills"
    );
}
