//! `domains::integrations::setup::detect` against a real temporary data directory and a real
//! `git init` working tree.
use super::run;
use crate::config::{Config, Paths};
use crate::test_common::{git_commit::commit_files, git_repo::init_repo};
use crate::utilities::context::Ctx;

/// A real git working tree carrying two real Rust files.
fn repo_with_sources() -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    init_repo(repo.path());
    commit_files(
        repo.path(),
        &[
            ("src/alpha.rs", "pub fn alpha() -> u8 { 1 }\n"),
            ("src/beta.rs", "pub fn beta(n: u8) -> u8 { n + 1 }\n"),
        ],
        "seed",
    );
    repo
}

#[test]
fn a_fresh_data_dir_is_probed_without_creating_the_database() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let paths = Paths::new(data.path().to_path_buf());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let detected = run(&mut ctx, work.path(), None).unwrap();

    assert!(!detected.db_writable, "a fresh dir has no writable db yet");
    assert!(
        !data.path().join("comemory.db").exists(),
        "detection must not create comemory.db"
    );
    assert!(detected.repo.is_none(), "a plain tempdir is not a git repo");
    assert!(detected.hooks_installed.is_empty());
    assert!(detected.index_status.is_none());
    assert_eq!(detected.doc_sources, 0);
    assert!(!detected.authenticated, "no auth.json was written");
}

#[test]
fn a_real_git_repo_is_detected_with_its_label_and_root() {
    let data = tempfile::tempdir().unwrap();
    let repo = repo_with_sources();
    let paths = Paths::new(data.path().to_path_buf());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let detected = run(&mut ctx, repo.path(), None).unwrap();

    let found = detected.repo.expect("a git working tree is a repo");
    assert!(found.is_git);
    assert!(
        !found.label.is_empty(),
        "repo label comes from the worktree"
    );
    assert!(
        std::path::Path::new(&found.root).exists(),
        "root must be a real absolute path, got {}",
        found.root
    );
    assert!(
        detected.hooks_installed.is_empty(),
        "a fresh repo has no comemory hooks"
    );
    assert!(detected.hooks_foreign.is_empty());
    assert!(
        detected.index_status.is_none(),
        "never indexed, so there is no repos row"
    );
}

#[test]
fn a_hand_written_hook_is_reported_as_foreign_not_installed() {
    let data = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    init_repo(repo.path());
    let hooks = repo.path().join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    std::fs::write(hooks.join("post-commit"), "#!/bin/sh\necho mine\n").unwrap();

    let paths = Paths::new(data.path().to_path_buf());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let detected = run(&mut ctx, repo.path(), None).unwrap();

    assert!(
        !detected.hooks_installed.iter().any(|h| h == "post-commit"),
        "someone else's hook is not a comemory hook"
    );
    assert_eq!(detected.hooks_foreign, vec!["post-commit".to_string()]);
}

#[test]
fn an_unknown_host_filter_yields_no_present_hosts() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let paths = Paths::new(data.path().to_path_buf());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    // A name outside `domains::integrations::install::HOSTS` filters everything out, so the
    // probe never shells out to an arbitrary program name.
    let detected = run(&mut ctx, work.path(), Some("definitely-not-a-host")).unwrap();
    assert!(detected.hosts_present.is_empty());
    assert!(detected.hosts_installed.is_empty());
}

#[test]
fn detection_lists_document_sources_without_reconciling_the_mirror() {
    let data = tempfile::tempdir().unwrap();
    let repo = repo_with_sources();
    let docs = tempfile::tempdir().unwrap();
    std::fs::write(docs.path().join("note.md"), "# note\n").unwrap();

    let paths = Paths::new(data.path().to_path_buf());
    let cfg = Config::defaults();
    paths.ensure_dirs().unwrap();
    let label = crate::domains::code::git_utils::repo_label_at(repo.path())
        .expect("the fixture is a real git working tree");

    // `sources.toml` is the durable registry and registering never writes the
    // SQLite mirror. That gap is what makes this discriminating: only a
    // reconciling listing would close it.
    crate::domains::documents::source::registry::Registry::new(paths.clone())
        .register(docs.path(), Some(label))
        .unwrap();

    let mut ctx = Ctx::lazy(&paths, &cfg);
    assert_eq!(
        crate::store::sources::count(ctx.conn().unwrap()).unwrap(),
        0,
        "registering writes sources.toml, never the mirror"
    );

    let detected = run(&mut ctx, repo.path(), None).unwrap();

    assert_eq!(
        detected.doc_sources, 0,
        "detection lists with `reconcile: false`, so it reports the mirror as it stands"
    );
    assert_eq!(
        crate::store::sources::count(ctx.conn().unwrap()).unwrap(),
        0,
        "a read-only probe must never rewrite the SQLite mirror from sources.toml"
    );
}
