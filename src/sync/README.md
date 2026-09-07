# sync/

**What belongs here:** cloud-sync client helpers used by `comemory auth`,
`link`, and `sync` — repo match-key classification against the GitHub App
allowlist, secret redaction before push, device credential file I/O, and the
cached allowlist bundle.

**What does NOT belong here:** sync log/state tables, HTTP routes, or import
rules — those live in `store/`, `api/`, and `serve/` respectively.

## Contents

| File | Primary item | Purpose |
| --- | --- | --- |
| `match_key.rs` | `classify_repo` | Normalize repo labels and git remotes; classify against the allowlist |
| `redact.rs` | `scan` | Curated secret scan (`rules.toml`) before enqueueing a push |
| `rules.toml` | — | Compile-time rule patterns (AWS, GitHub/GitLab/Slack tokens, JWT, PEM, …) |
| `auth_file.rs` | `AuthFile` | Load/save `$COMEMORY_DATA_DIR/auth.json` (0600 on unix) |
| `allowlist_cache.rs` | `AllowlistCache` | Load/save `allowlist.json`, freshness TTL, classify wrapper |
| `client.rs` | platform HTTP | Device mint (`/v1/device/…`), sync status allowlist, enveloped sync I/O |
| `push.rs` / `pull.rs` | `run_push` / `run_pull` | Allowlist-filtered push and cursored pull |
| `verify.rs` | `verify_manifests` | Manifest compare + pull/push repair (AC-9) |
| `auto.rs` | auto-sync hooks | Best-effort after-save / verify schedules |

Submodules are declared from `src/sync.rs`; callers import `comemory::sync::…`.
