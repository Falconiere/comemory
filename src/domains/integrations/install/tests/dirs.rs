//! `mirror_dir` behavior: fresh copy, idempotent rerun, refusal on foreign
//! edits and symlinks, executable bits for shell scripts.
use super::mirror_dir;

fn stage_source(task: &std::path::Path) -> std::path::PathBuf {
    let src = task.join("src");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(src.join("a.txt"), "alpha").unwrap();
    std::fs::write(src.join("sub").join("b.sh"), "#!/bin/sh\n").unwrap();
    src
}

#[test]
fn mirror_copies_a_fresh_tree_with_executable_scripts() {
    let task = tempfile::tempdir().unwrap();
    let src = stage_source(task.path());
    let dst = task.path().join("dst");

    mirror_dir(&src, &dst).unwrap();

    assert_eq!(std::fs::read(dst.join("a.txt")).unwrap(), b"alpha");
    assert_eq!(
        std::fs::read(dst.join("sub").join("b.sh")).unwrap(),
        b"#!/bin/sh\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(dst.join("sub").join("b.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "shell scripts stay executable");
    }
}

#[test]
fn mirror_rerun_over_identical_content_is_a_noop() {
    let task = tempfile::tempdir().unwrap();
    let src = stage_source(task.path());
    let dst = task.path().join("dst");
    mirror_dir(&src, &dst).unwrap();
    let before = std::fs::read(dst.join("a.txt")).unwrap();

    mirror_dir(&src, &dst).unwrap();

    assert_eq!(std::fs::read(dst.join("a.txt")).unwrap(), before);
}

#[test]
fn mirror_refuses_differing_content_and_leaves_it_untouched() {
    let task = tempfile::tempdir().unwrap();
    let src = stage_source(task.path());
    let dst = task.path().join("dst");
    mirror_dir(&src, &dst).unwrap();
    std::fs::write(dst.join("a.txt"), "operator edits").unwrap();

    let err = mirror_dir(&src, &dst).unwrap_err().to_string();

    assert!(err.contains("differs"), "unexpected error: {err}");
    assert_eq!(std::fs::read(dst.join("a.txt")).unwrap(), b"operator edits");
}

#[cfg(unix)]
#[test]
fn mirror_refuses_a_symlinked_destination() {
    let task = tempfile::tempdir().unwrap();
    let src = stage_source(task.path());
    let elsewhere = task.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let dst = task.path().join("dst");
    std::os::unix::fs::symlink(&elsewhere, &dst).unwrap();

    let err = mirror_dir(&src, &dst).unwrap_err().to_string();

    assert!(err.contains("refusing symlink"), "unexpected error: {err}");
    assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
}
