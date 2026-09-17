# sync/

**What belongs here:** cloud-sync client helpers used by `comemory auth` and
`comemory sync` — the org-scoped credential file, secret redaction before
push, the local `[sync] skip_repos` filter, the platform HTTP client, the
push / pull / verify / first-sync runs, the code-index push, and the
user-level sync daemon.

**What does NOT belong here:** sync log/state tables, HTTP routes, or import
rules — those live in `store/`, `api/`, and `serve/` respectively. The
per-repo GitHub App allowlist is gone entirely: organization membership is the
platform's gate, and `skip_repos` is the only filter left on this side. The
rule that an unlabelled memory stayed local is gone too — `repo` comes from the
cwd's git repository, so it made sync eligibility depend on which directory
`comemory save` ran in (`2026-09-14-sync-everything-realtime-design.md`).

## Contents

| File | Primary item | Purpose |
| --- | --- | --- |
| `skip_repos.rs` | `SkipMatcher` | Normalize a repo label; match it against the `[sync] skip_repos` globs — the one client-side filter left |
| `push_on_save.rs` | `after_write_best_effort` | Drain the outbox inline after a local write, bounded by `[sync] push_on_save_timeout`; never fails a write, never throws |
| `redact.rs` | `scan` | Curated secret scan (`rules.toml`) before enqueueing a push |
| `rules.toml` | — | Compile-time rule patterns (AWS, GitHub/GitLab/Slack tokens, JWT, PEM, …) |
| `auth_file.rs` | `AuthFile` | Load/save `$COMEMORY_DATA_DIR/auth.json` v2 (0600 on unix); reject a v1 file |
| `client.rs` | platform HTTP | Enveloped sync I/O over reqwest+rustls; sends no workspace header. `ws_ticket` / `channel_url` are the workspace channel's two client calls |
| `client_code.rs` | `fetch_code_manifest` | The two calls behind the code push — `GET /v1/sync/code/manifest?repo=` and `POST /v1/sync/code/import` — over `client.rs`'s base URL, credential and envelope |
| `code.rs` | `run_code_push` | Push the code index of every indexed repo (minus `skip_repos`, off with `[sync] code_index = false`): read the workspace manifest, diff by blob OID, post only what differs. `run_code_push_if_moved` (daemon, `index-code` tail) first checks the `code_sync:<repo>` cursor and stays silent when nothing moved; `project_file` is the one place a snippet-free file entry is built |
| `code_plan.rs` | `plan` | The pure diff (changed / removed / send-cochange / head-moved) and the batching (500 files, ~1 MiB) — no store, no network |
| `push.rs` / `pull.rs` | `run_push` / `run_pull` | Push filtered by `skip_repos` alone (backfills missing `sync_log` rows first; does not advance `pushed_seq` on all-`repo_not_allowed` batches), and cursored pull. `run_push_with_timeout` is the same walk under the inline push's smaller budget |
| `initial.rs` | `run_initial_sync` | Exhaustive pull-then-push that `auth login` runs before returning, then the code push — so the console's graph fills in from the first login |
| `verify.rs` | `verify_manifests` | Manifest compare + pull/push repair (AC-9) |
| `daemon.rs` | `run_foreground` | Periodic pull+push+verify loop the OS supervisor keeps alive — opt-in (`auth login --daemon`) since a save pushes itself |
| `daemon_unit.rs` | `install` / `status` | launchd / systemd --user unit lifecycle |
| `daemon_templates.rs` | plist / unit text | Rendered unit bodies |

Submodules are declared from `src/sync.rs`; callers import `comemory::domains::sync::…`.
