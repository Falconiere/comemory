# api/repos/

**What belongs here:** the git half of `api::repos` — HEAD comparison
against `repo_marker.last_head`, remote/branch lookup, and the changed-file
count for a stale repo. Split out because staleness resolution reuses
`git_utils` and its degrade-never-propagate error contract is distinct
enough from the SQL join to earn its own file (Binding Rule 4 — the size
ceiling — plus Binding Rule 3, one responsibility per file).

**What does NOT belong here:** the command's entry point, `Request` /
`Response` / `Row` shape, and the `repo_marker` join, which stay in
`src/api/repos.rs`.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `git_state.rs` | `resolve` | Compare a repo's stored `last_head` against its working tree's current git HEAD (`git_utils::current_head`), resolve `remote`/`branch`, and count changed files when stale. Every git failure degrades to `status: "unknown"` rather than propagating an `Err` — `comemory repos` always exits 0 against a resolvable database |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/api/repos.rs`
(`pub mod <name>;`) and callers import concrete paths.
