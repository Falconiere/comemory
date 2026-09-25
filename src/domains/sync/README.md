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
`code_sync`, `schema_sync`). The platform's repository policy is authoritative.
The client resolves each local label to an approved canonical GitHub
`owner/name` through a current checkout remote or an administrator-confirmed
mapping; ambiguous, unsupported and unlabelled entries remain local.

## Contents

### Credentials and login

| File | Primary item | Purpose |
| --- | --- | --- |
| `auto.rs` | `run_auto` / `run_pass` | `comemory sync --action auto`, the pass git hooks and the agent `SessionStart` hook fire from any cwd: index the triggering checkout, refresh every stale hooked repo, then (logged in) drain the key through `drain::drain` until a pass ends without `more`, and — on a legacy key of a managed origin — push moved code. Serialized on `sync.lock`; extra triggers coalesce through the one-slot `sync-auto.queue` |
| `auth_file.rs` | `AuthFile` | Load/save `$COMEMORY_DATA_DIR/auth.json` v2 (0600 on unix); require a new login for v1 files and v2-stamped files lacking organization identity. `load_usable` reports those stale credentials as absent to `setup` without hiding corrupt JSON; it never mints, dials, or starts anything |
| `cloud.rs` + [`cloud/`](cloud/README.md) | `login` | API base URL resolution and the RFC 8628 device flow that mints the org key |
| `login.rs` | `establish` | The three `comemory auth` sequences — login, status probe, logout — with the progress writer injected and no rendering. A logout (`forget`) or a login that changes the key stamps every still-owed change with the key being left |

### The platform protocol

| File | Primary item | Purpose |
| --- | --- | --- |
| `client.rs` | platform HTTP | Enveloped sync I/O over reqwest+rustls; sends no workspace header. `ws_ticket` / `channel_url` are the workspace channel's two client calls |
| `client_code.rs` | `fetch_code_manifest` | The two calls behind the code push — `GET /v1/sync/code/manifest?repo=` and `POST /v1/sync/code/import` — over `client.rs`'s base URL, credential and envelope |
| `client_policy.rs` | `fetch` | Fetch and decode the authoritative repository policy from sync status |
| `client_protocol.rs` | `validate_response` | Managed data-request protocol/revision headers and fail-closed response checks |
| `repository_identity.rs` | `canonical_github_repository` | Strict HTTPS/SSH/SCP `github.com` remote parsing into lowercase `owner/name` identities |
| `repository_policy.rs` | `RepositoryPolicy` | Validate the server policy, resolve local checkout identities and mappings, and reconcile persisted fingerprints |
| `exchange.rs` + [`exchange/`](exchange/README.md) | `import::run` | The *server* side: wire models plus the `changes` / `manifest` / `import` / code-import cores the `serve` routes call. Its import path writes the `replica-v1` journal too, so a legacy push and a replica push converge on one history |
| `replica.rs` + [`replica/`](replica/README.md) | `accept::run` | The `replica-v1` contract and its engine halves: the versioned envelope, the acceptance decision, the materialize-journal-receipt transaction, the `changes` / `manifest` / `events` reads, staged revisions, and journal seeding |

### Directions of travel

| File | Primary item | Purpose |
| --- | --- | --- |
| `drain.rs` + [`drain/`](drain/README.md) | `drain` | The exchange client (#255): one entry point every caller drains through — session open and protocol negotiation, the `replica-v1` pass and the legacy pass under one loop and budget, holds, replay, verify and the `exchange` status block |
| `push.rs` / `pull.rs` | `batch` / `page` | The legacy pass's wire: one push batch of approved local log entries (the `repositories` map only for a managed origin), and one pull page imported entry by entry — never past an entry the import did not apply, refusals kept as `legacy` holds. A server repository rejection stops the push before cursor advancement |
| `push_on_save.rs` | `after_write_best_effort` | Push the outbox inline after a local write — one push-only pass, bounded by `[sync] push_on_save_timeout` with no in-pass retries; skipped when a pass holds `sync.lock` (that pass sends it). Never fails a write, never throws. Called from the CLI seam only, so an HTTP write starts no outward sync |
| `code.rs` | `run_code_push` | Push only indexed checkouts whose current GitHub origin resolves to an approved canonical identity (minus `skip_repos`, invalid roots and worktrees; off with `[sync] code_index = false`). The wire carries the canonical name while local rows keep their label. `run_code_push_if_moved` first checks the policy-aware cursor |
| `code_repo_push.rs` | `push_repo` | Diff and upload one local code index under its canonical platform identity, then persist its local-label cursor |
| `code_plan.rs` | `plan` | The pure diff (changed / removed / send-cochange / head-moved) and the batching (500 files, ~1 MiB) — no store, no network |
| `vector_rule.rs` | `decide` | The one verdict both import wires apply to an arriving embedding: usable only from this engine's model at its `vec0` dimension, and a refusal stores the memory text anyway and records the id as needing an embedding |
| `verify.rs` | `verify` | Compare and repair manifests: per kind over the key's bindings on `replica-v1` (`drain::verify`), over the locally authorized subset on a legacy key (repair re-runs the drain from zero cursors). The caller holds `sync.lock` |
| `initial.rs` | `run_initial_sync` | The drain `auth login` runs before returning (pull first, until caught up), then — on a legacy key of a managed origin — the code push, so the console's graph fills in from the first login |
| `manual.rs` | `open_session` / `run` | What one `comemory sync` run does: the credential-then-store session it opens, then — under `sync.lock` — a secret override, the stale-hooked-repo refresh, the drain of the requested legs, and on a legacy key of a managed origin the code push |

### Filters, following, and the resident loop

| File | Primary item | Purpose |
| --- | --- | --- |
| `redact.rs` | `scan` | Curated secret scan (`rules.toml`) before enqueueing a push |
| `rules.toml` | — | Compile-time rule patterns (AWS, GitHub/GitLab/Slack tokens, JWT, PEM, …), loaded by `redact.rs` through `include_str!` |
| `skip_repos.rs` | `SkipMatcher` | Normalize a repo label and apply the operator's additional `[sync] skip_repos` withholding globs |
| `watch.rs` | `follow` | Hold the workspace channel and drain the pull direction on every nudge (under `sync.lock`, until a pass ends without `more`), with full-jitter reconnect. Emits `WatchEvent`s to a caller-supplied callback and takes the blocking-I/O escape hatch as an `OffRuntime`, so it renders nothing and names no delivery module |
| `daemon.rs` | `run_foreground` | Periodic loop the OS supervisor keeps alive — each cycle is `auto::run_pass` under `sync.lock`, plus a verify when due — opt-in (`auth login --daemon`) since a save pushes itself and hooks fire passes |
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
