//! `link_skills` against the real embedded bundle: fresh links, idempotent
//! reruns, an upgrade repointing a previous version's links, refusal of
//! anything comemory did not write, and the wrappers finding their libs
//! through the link.
use std::path::{Path, PathBuf};

use super::link_skills;

/// Extract the real embedded bundle at `<task>/integrations/<version>`.
fn extract_bundle(task: &Path, version: &str) -> PathBuf {
    let root = task.join("integrations").join(version);
    super::super::bundle::extract(&root).unwrap();
    root
}

fn skill_names(root: &Path) -> Vec<std::ffi::OsString> {
    let mut names: Vec<_> = std::fs::read_dir(root.join("plugins/comemory/skills"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    names
}

#[test]
fn links_every_bundled_skill_and_reruns_as_a_noop() {
    let task = tempfile::tempdir().unwrap();
    let root = extract_bundle(task.path(), "1.0.0");
    let target = task.path().join("host/skills");

    link_skills(&root, &target).unwrap();

    let names = skill_names(&root);
    assert!(!names.is_empty());
    for name in &names {
        let link = target.join(name);
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            root.join("plugins/comemory/skills").join(name),
            "{} must link into the bundle",
            link.display()
        );
        assert!(link.join("SKILL.md").is_file());
    }
    link_skills(&root, &target).unwrap();
    assert_eq!(
        std::fs::read_dir(&target).unwrap().count(),
        names.len(),
        "a rerun leaves no temporary links behind"
    );
}

#[test]
fn an_upgrade_repoints_links_from_a_previous_bundle_version() {
    let task = tempfile::tempdir().unwrap();
    let old = extract_bundle(task.path(), "1.0.0");
    let new = extract_bundle(task.path(), "2.0.0");
    let target = task.path().join("host/skills");
    link_skills(&old, &target).unwrap();

    link_skills(&new, &target).unwrap();

    for name in skill_names(&new) {
        assert_eq!(
            std::fs::read_link(target.join(&name)).unwrap(),
            new.join("plugins/comemory/skills").join(&name)
        );
    }
}

#[test]
fn ownership_holds_however_the_data_dir_was_spelled() {
    let task = tempfile::tempdir().unwrap();
    let real = task.path().join("real");
    let old = extract_bundle(&real, "1.0.0");
    let new = extract_bundle(&real, "2.0.0");
    // The previous install reached the data dir through a symlinked path,
    // the way `/var/folders` reaches `/private/var/folders` on macOS.
    let alias = task.path().join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let old_via_alias = alias.join(old.strip_prefix(&real).unwrap());
    let target = task.path().join("host/skills");
    link_skills(&old_via_alias, &target).unwrap();

    link_skills(&new, &target).unwrap();

    assert_eq!(
        std::fs::read_link(target.join("agent-memory")).unwrap(),
        new.join("plugins/comemory/skills/agent-memory")
    );
}

#[test]
fn a_dangling_link_to_a_deleted_version_spelled_through_an_alias_is_still_ours() {
    let task = tempfile::tempdir().unwrap();
    let real = task.path().join("real");
    let old = extract_bundle(&real, "1.0.0");
    let new = extract_bundle(&real, "2.0.0");
    let alias = task.path().join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let target = task.path().join("host/skills");
    link_skills(&alias.join(old.strip_prefix(&real).unwrap()), &target).unwrap();
    // The old version is gone: its links dangle, spelled through the alias.
    std::fs::remove_dir_all(&old).unwrap();

    link_skills(&new, &target).unwrap();

    assert_eq!(
        std::fs::read_link(target.join("agent-memory")).unwrap(),
        new.join("plugins/comemory/skills/agent-memory")
    );
}

#[test]
fn refuses_a_real_directory_or_a_foreign_link_and_changes_nothing() {
    let task = tempfile::tempdir().unwrap();
    let root = extract_bundle(task.path(), "1.0.0");
    let target = task.path().join("host/skills");
    let names = skill_names(&root);

    // A real directory under the last skill name: the operator's own skill.
    let last = target.join(names.last().unwrap());
    std::fs::create_dir_all(&last).unwrap();
    let err = link_skills(&root, &target).unwrap_err().to_string();
    assert!(err.contains("not a comemory skill link"), "{err}");
    assert!(
        target.join(&names[0]).symlink_metadata().is_err(),
        "every entry is checked before any link is written"
    );
    std::fs::remove_dir(&last).unwrap();

    // A link to somewhere outside comemory's integrations dir.
    let elsewhere = task.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &last).unwrap();
    let err = link_skills(&root, &target).unwrap_err().to_string();
    assert!(err.contains("which comemory did not write"), "{err}");
    assert_eq!(std::fs::read_link(&last).unwrap(), elsewhere);
}

#[test]
fn wrappers_find_their_bundle_libs_through_the_link() {
    let task = tempfile::tempdir().unwrap();
    let root = extract_bundle(task.path(), "1.0.0");
    let target = task.path().join("host/skills");
    link_skills(&root, &target).unwrap();

    // skills.sh exits 1 with "lib not found" when `../../../lib` does not
    // resolve; through the link it reaches the bundle's lib and prints usage.
    let output = std::process::Command::new("bash")
        .arg(target.join("project-skills/scripts/skills.sh"))
        .current_dir(task.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stderr.contains("lib not found"), "{stderr}");
    assert!(
        stdout.contains("Usage: skills.sh") || stderr.contains("Usage: skills.sh"),
        "the wrapper must reach its usage text: {stdout}{stderr}"
    );

    // Outside a git repo comemory.sh scopes to "unknown" and names the
    // cause: a missing repo-scope lib (broken install) or no repo. A stub
    // `comemory` on PATH gets it past the CLI check to that warning.
    let bin = task.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("comemory"), "#!/bin/sh\nexit 0\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(bin.join("comemory"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }
    let output = std::process::Command::new("bash")
        .arg(target.join("agent-memory/scripts/comemory.sh"))
        .arg("list")
        .current_dir(&bin)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("GIT_CEILING_DIRECTORIES", task.path())
        .env_remove("TOOLU_COMEMORY_REPO")
        .env_remove("MY_CLAUDE_COMEMORY_REPO")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(!stderr.contains("repo-scope lib not found"), "{stderr}");
    assert!(stderr.contains("no git repo"), "{stderr}");
}
