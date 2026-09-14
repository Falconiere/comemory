# sync/

**What belongs here:** cloud-sync client helpers used by `comemory auth` and
`comemory sync` — the org-scoped credential file, secret redaction before
push, the local `[sync] skip_repos` filter, the platform HTTP client, the
push / pull / verify / first-sync runs, and the user-level sync daemon.

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
| `client.rs` | platform HTTP | Enveloped sync I/O over reqwest+rustls; sends no workspace header |
| `push.rs` / `pull.rs` | `run_push` / `run_pull` | Push filtered by label + `skip_repos`, and cursored pull |
| `initial.rs` | `run_initial_sync` | Exhaustive pull-then-push that `auth login` runs before returning |
| `verify.rs` | `verify_manifests` | Manifest compare + pull/push repair (AC-9) |
| `daemon.rs` | `run_foreground` | Periodic pull+push+verify loop the OS supervisor keeps alive |
| `daemon_unit.rs` | `install` / `status` | launchd / systemd --user unit lifecycle |
| `daemon_templates.rs` | plist / unit text | Rendered unit bodies |

Submodules are declared from `src/sync.rs`; callers import `comemory::sync::…`.
