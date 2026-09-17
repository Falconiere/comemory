# `domains/sync/`

**What belongs here:** keeping this machine and the organization's platform
holding the same memories and code index — the org-scoped credential and the
device login that mints it, the wire protocol's client *and* server halves,
push / pull / verify / first-sync, the code-index push, the secret scan and
skip filter that decide what may leave, the workspace channel, the opt-in
daemon, and the Git synchronization of the data directory itself.

**What does NOT belong here:** delivery or storage. Clap flags, prompts,
process launch and every rendered line stay in [`cli/`](../../cli/README.md)
(`cli/auth.rs`, `cli/sync.rs`, `cli/watch.rs` and the two `*_render.rs`
modules); loopback HTTP routing, guards and SSE stay in
[`serve/`](../../serve/README.md); every SQL string and driver import stays in
[`store/`](../../store/README.md) (`sync_log`, `sync_state`, `sync_binding`,
`code_sync`, `schema_sync`). The per-repo GitHub App allowlist is gone
entirely: organization membership is the platform's gate, and `skip_repos` is
the only filter left on this side. The rule that an unlabelled memory stayed
local is gone too — `repo` comes from the cwd's git repository, so it made sync
eligibility depend on which directory `comemory save` ran in
(`2026-09-14-sync-everything-realtime-design.md`).

## Contents

### Credentials and login

| File | Primary item | Purpose |
| --- | --- | --- |
| `auth_file.rs` | `AuthFile` | Load/save `$COMEMORY_DATA_DIR/auth.json` v2 (0600 on unix); reject a v1 file. `load_usable` is the read-only probe `setup` detection uses — it never mints, dials, or starts anything |
| `cloud.rs` + [`cloud/`](cloud/README.md) | `login` | API base URL resolution and the RFC 8628 device flow that mints the org key |
| `login.rs` | `establish` | The three `comemory auth` sequences — login, status probe, logout — with the progress writer injected and no rendering |

### The platform protocol

| File | Primary item | Purpose |
| --- | --- | --- |
| `client.rs` | platform HTTP | Enveloped sync I/O over reqwest+rustls; sends no workspace header. `ws_ticket` / `channel_url` are the workspace channel's two client calls |
| `client_code.rs` | `fetch_code_manifest` | The two calls behind the code push — `GET /v1/sync/code/manifest?repo=` and `POST /v1/sync/code/import` — over `client.rs`'s base URL, credential and envelope |
| `exchange.rs` + [`exchange/`](exchange/README.md) | `import::run` | The *server* side: wire models plus the `changes` / `manifest` / `import` / code-import cores the `serve` routes call |

### Directions of travel

| File | Primary item | Purpose |
| --- | --- | --- |
| `push.rs` / `pull.rs` | `run_push` / `run_pull` | Push filtered by `skip_repos` alone (backfills missing `sync_log` rows first; does not advance `pushed_seq` on all-`repo_not_allowed` batches), and cursored pull. `run_push_with_timeout` is the same walk under the inline push's smaller budget |
| `push_on_save.rs` | `after_write_best_effort` | Drain the outbox inline after a local write, bounded by `[sync] push_on_save_timeout`; never fails a write, never throws. Called from the CLI seam only, so an HTTP write starts no outward sync |
| `code.rs` | `run_code_push` | Push the code index of every indexed repo (minus `skip_repos`, off with `[sync] code_index = false`): read the workspace manifest, diff by blob OID, post only what differs. `run_code_push_if_moved` (daemon, `index-code` tail) first checks the `code_sync:<repo>` cursor and stays silent when nothing moved; `project_file` is the one place a snippet-free file entry is built |
| `code_plan.rs` | `plan` | The pure diff (changed / removed / send-cochange / head-moved) and the batching (500 files, ~1 MiB) — no store, no network |
| `verify.rs` | `verify_manifests` | Manifest compare + pull/push repair (AC-9) |
| `initial.rs` | `run_initial_sync` | Exhaustive pull-then-push that `auth login` runs before returning, then the code push — so the console's graph fills in from the first login |
| `manual.rs` | `open_session` / `run_all` | What one `comemory sync` run does: the credential-then-store session it opens, and the three composite action sequences |

### Filters, following, and the resident loop

| File | Primary item | Purpose |
| --- | --- | --- |
| `redact.rs` | `scan` | Curated secret scan (`rules.toml`) before enqueueing a push |
| `rules.toml` | — | Compile-time rule patterns (AWS, GitHub/GitLab/Slack tokens, JWT, PEM, …), loaded by `redact.rs` through `include_str!` |
| `skip_repos.rs` | `SkipMatcher` | Normalize a repo label; match it against the `[sync] skip_repos` globs — the one client-side filter left |
| `watch.rs` | `follow` | Hold the workspace channel and pull on every nudge, with full-jitter reconnect. Emits `WatchEvent`s to a caller-supplied callback and takes the blocking-I/O escape hatch as an `OffRuntime`, so it renders nothing and names no delivery module |
| `daemon.rs` | `run_foreground` | Periodic pull+push+verify loop the OS supervisor keeps alive — opt-in (`auth login --daemon`) since a save pushes itself |
| `daemon_unit.rs` | `install` / `status` | launchd / systemd --user unit lifecycle |
| `daemon_templates.rs` | plist / unit text | Rendered unit bodies |

### A different kind of sync

| File | Primary item | Purpose |
| --- | --- | --- |
| `memory_store.rs` + [`memory_store/`](memory_store/README.md) | `sync` | Git synchronization of the data directory's own work tree — commit, pull, push — with its own `store-sync` job and log contract. Not platform push/pull: different remote, different credentials, different cursors |

Tests live beside the modules under `tests/`. Submodules are declared from
`src/domains/sync.rs`; in-crate callers import `crate::domains::sync::…`, and
`comemory::sync` / `comemory::cloud` remain crate-root aliases for external
consumers only.

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel.
