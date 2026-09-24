#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    // Shared across several suites; each uses a different subset.
    dead_code
)]
//! A small real TypeScript repo, indexed by a real `index-code`, for the
//! code-index sync suites: three files whose relative imports resolve onto
//! each other, so the projection carries real `imports` edges, plus the one
//! commit that gives the co-change miner something to count.
//!
//! Pairs with `git_repo.rs` / `git_commit.rs` the same way `git_sample.rs`
//! does: a binary that includes this file must include both.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use comemory::config::{Config, Paths};
use comemory::domains::sync::exchange::CodeFileWire;
use comemory::store::{Connection, indexed_files};
use comemory::utilities::context::Ctx;

/// The repo label every fixture row lands under.
pub const REPO: &str = "scratch";
/// Canonical GitHub identity resolved from the fixture checkout's origin.
pub const CANONICAL_REPO: &str = "falconiere/comemory";

/// A source line that must never appear on the wire — the snippet-free
/// assertion greps every recorded request body for it.
pub const SECRET_BODY_LINE: &str = "const onlyInSource = 'never-on-the-wire';";

/// `<root>/scratch` with `src/a.ts`, `src/b.ts` (imports `./a`) and
/// `src/c.ts` (imports `./a` and `./b`), committed once. Returns the tree.
pub fn write_ts_repo(root: &Path) -> PathBuf {
    let repo = root.join(REPO);
    let remote = format!("git@github.com:{CANONICAL_REPO}.git");
    super::git_repo::init_repo(&repo);
    super::git_repo::run_git(&repo, &["remote", "add", "origin", &remote]);
    super::git_commit::commit_files(
        &repo,
        &[
            (
                "src/a.ts",
                &format!(
                    "export function alpha(): number {{\n  {SECRET_BODY_LINE}\n  return 1;\n}}\n"
                ),
            ),
            (
                "src/b.ts",
                "import { alpha } from './a';\nexport function beta(): number {\n  return alpha() + 1;\n}\n",
            ),
            (
                "src/c.ts",
                "import { alpha } from './a';\nimport { beta } from './b';\nexport function gamma(): number {\n  return alpha() + beta();\n}\n",
            ),
        ],
        "init",
    );
    repo
}

/// Run a real `index-code` over `tree` into `paths`' database.
pub fn index(paths: &Paths, cfg: &Config, conn: &mut Connection, tree: &Path) {
    index_mode(
        paths,
        cfg,
        conn,
        tree,
        comemory::domains::code::index_code::IndexMode::Incremental,
    );
}

/// [`index`] with an explicit mode (`Full` re-walks every file and drops
/// cursors for files that no longer exist).
pub fn index_mode(
    paths: &Paths,
    cfg: &Config,
    conn: &mut Connection,
    tree: &Path,
    mode: comemory::domains::code::index_code::IndexMode,
) {
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    comemory::domains::code::index_code::run(
        &mut ctx,
        comemory::domains::code::index_code::Request {
            repo: REPO.into(),
            path: tree.to_string_lossy().into_owned(),
            mode,
        },
    )
    .expect("index-code");
}

/// Every indexed file of [`REPO`] as the push would send it, by path.
pub fn projection(conn: &Connection) -> Vec<CodeFileWire> {
    let files = indexed_files::list_for_repo(conn, REPO).expect("indexed files");
    let known: BTreeSet<&str> = files.iter().map(|(path, _)| path.as_str()).collect();
    files
        .iter()
        .map(|(path, _)| {
            comemory::domains::sync::code::project_file(conn, REPO, path, &known).expect("project")
        })
        .collect()
}

/// Install comemory's real hooks into `tree` with hook *execution* switched
/// off (`core.hooksPath=/dev/null`): the repo counts as hooked — the hooks
/// directory is read from the common dir, not `core.hooksPath` — but no
/// commit launches whatever `comemory` is on this host's `PATH` against its
/// real data directory.
pub fn install_inert_hooks(paths: &Paths, cfg: &Config, tree: &Path) {
    super::git_repo::run_git(tree, &["config", "core.hooksPath", "/dev/null"]);
    let mut ctx = Ctx::lazy(paths, cfg);
    comemory::domains::code::install_hooks::run(
        &mut ctx,
        comemory::domains::code::install_hooks::Request {
            repo: tree.display().to_string(),
            force: false,
        },
    )
    .expect("install hooks");
}

/// Commit `files` into `tree` — a HEAD move no hook saw once
/// [`install_inert_hooks`] ran — and return the new HEAD oid.
pub fn commit_hookless(tree: &Path, files: &[(&str, &str)], msg: &str) -> String {
    super::git_commit::commit_files(tree, files, msg);
    comemory::domains::code::git_utils::current_head(tree).expect("head")
}
