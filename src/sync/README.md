# sync/

**What belongs here:** cloud-sync client helpers used by `comemory auth` and
`comemory sync` — the org-scoped credential file, secret redaction before
push, the local `[sync] skip_repos` filter, the platform HTTP client, and the
push / pull / verify / first-sync runs over it.

**What does NOT belong here:** sync log/state tables, HTTP routes, or import
rules — those live in `store/`, `api/`, and `serve/` respectively. The
per-repo GitHub App allowlist is gone entirely: organization membership is the
platform's gate, and `skip_repos` is the only filter left on this side.

## Contents

| File | Primary item | Purpose |
| --- | --- | --- |
| `skip_repos.rs` | `SkipMatcher` | Normalize a repo label; match it against the `[sync] skip_repos` globs |
| `redact.rs` | `scan` | Curated secret scan (`rules.toml`) before enqueueing a push |
| `rules.toml` | — | Compile-time rule patterns (AWS, GitHub/GitLab/Slack tokens, JWT, PEM, …) |
| `auth_file.rs` | `AuthFile` | Load/save `$COMEMORY_DATA_DIR/auth.json` v2 (0600 on unix); reject a v1 file |
| `client.rs` | platform HTTP | Org-key mint (`/v1/device/…`) and enveloped sync I/O; sends no workspace header |
| `push.rs` / `pull.rs` | `run_push` / `run_pull` | Push filtered by label + `skip_repos`, and cursored pull |
| `initial.rs` | `run_initial_sync` | The pull-then-push `comemory auth login` runs before returning |
| `verify.rs` | `verify_manifests` | Manifest compare + pull/push repair (AC-9) |
| `auto.rs` | `after_save_best_effort` | The after-save push, returned as a joinable `AutoPush` |

Submodules are declared from `src/sync.rs`; callers import `comemory::sync::…`.
