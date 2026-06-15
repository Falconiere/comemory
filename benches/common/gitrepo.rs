//! Real on-disk git repo + matching `code_symbols` rows for the graph
//! benches. Shells out to the real `git` binary (mirroring
//! `tests/common/git_repo.rs`) so `mine_cochange` / `materialize` walk the
//! same `.git/` layout a user repo has. Only `benches/graph.rs` includes
//! this file.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use comemory::store::code_row::{self, CodeSymbolRow};
use rusqlite::Connection;

/// Number of source files each commit touches (≥2, else `mine_cochange`
/// yields zero co-change pairs and the bench measures nothing).
const FILES_PER_COMMIT: usize = 3;

/// Run a git subcommand in `repo`, panicking on failure — a broken git
/// environment is a broken bench, not a measurable path.
fn run_git(repo: &Path, args: &[&str]) {
    let st = Command::new("git")
        .current_dir(repo)
        .args(args)
        .status()
        .expect("invoke git");
    assert!(st.success(), "git {args:?}");
}

/// The repo-relative path of synthetic source file `f`.
fn file_name(f: usize) -> String {
    format!("src/mod{f}.rs")
}

/// Build a real git repo with `commits` commits, each touching
/// `FILES_PER_COMMIT` rotating source files. Returns the tempdir (kept
/// alive by the caller), the repo root, and the known-file set the miner
/// filters co-change pairs against.
pub fn build_git_repo(commits: usize) -> (tempfile::TempDir, PathBuf, HashSet<String>) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    run_git(&root, &["init", "-q"]);
    run_git(&root, &["config", "user.email", "bench@example.com"]);
    run_git(&root, &["config", "user.name", "bench"]);
    run_git(&root, &["checkout", "-q", "-b", "main"]);

    let total_files = commits + FILES_PER_COMMIT;
    let mut known = HashSet::new();
    for c in 0..commits {
        for k in 0..FILES_PER_COMMIT {
            let f = (c + k) % total_files;
            let rel = file_name(f);
            let full = root.join(&rel);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(&full, format!("fn f{f}() {{ let v = {c}; }}\n")).unwrap();
            known.insert(rel);
        }
        run_git(&root, &["add", "-A"]);
        run_git(&root, &["commit", "-q", "-m", &format!("commit {c}")]);
    }
    (tmp, root, known)
}

/// Seed `code_symbols` rows for the repo's source files and return an
/// `imports_by_file` map so `materialize` has real inputs: it reads
/// `code_symbols.path` for the repo and resolves the import edges. Each
/// file imports the next via a relative module path.
pub fn seed_repo_symbols(
    conn: &Connection,
    repo: &str,
    repo_root: &Path,
) -> BTreeMap<String, Vec<String>> {
    let mut imports = BTreeMap::new();
    let paths = sorted_repo_files(repo_root);
    for (idx, path) in paths.iter().enumerate() {
        let symbol = format!("f{idx}");
        code_row::insert(
            conn,
            &CodeSymbolRow {
                repo,
                path,
                blob_oid: "oid",
                symbol: &symbol,
                kind: "function",
                lang: "rust",
                line_start: 1,
                line_end: 5,
                snippet: "fn body() {}",
                simhash: 0,
                parent_id: None,
            },
        )
        .unwrap();
        let next = &paths[(idx + 1) % paths.len()];
        imports.insert(path.clone(), vec![format!("crate::{}", module_of(next))]);
    }
    imports
}

/// Repo-relative `src/*.rs` paths under `repo_root`, sorted for determinism.
fn sorted_repo_files(repo_root: &Path) -> Vec<String> {
    let src = repo_root.join("src");
    let mut out: Vec<String> = std::fs::read_dir(&src)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| format!("src/{}", e.file_name().to_string_lossy()))
        .collect();
    out.sort();
    out
}

/// Module name (`mod3`) for a repo-relative `src/mod3.rs` path.
fn module_of(path: &str) -> String {
    path.trim_start_matches("src/")
        .trim_end_matches(".rs")
        .to_string()
}
