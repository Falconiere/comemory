# `domains/sync/daemon/`

**What belongs here:** the required resident sync coordinator
(`comemory sync daemon run`) and everything that finds, verifies, starts and
repairs it. Design: `docs/designs/2026-09-25-required-sync-daemon.md`.

**What does NOT belong here:** clap flags, the preflight classification and
every rendered line (`cli/sync_daemon.rs`, `cli/daemon_preflight.rs`); the
pass itself (`domains/sync/auto.rs`, `domains/sync/drain/`); unit file text
(`domains/sync/daemon_templates.rs`, `daemon_unit.rs`). Unix only — every
published target is.

## Contents

### Protocol and discovery

| File | Primary item | Purpose |
| --- | --- | --- |
| `identity.rs` | `BinaryIdentity` | Canonical data directory, its short id, and the binary a coordinator runs |
| `control.rs` | `Op` | The control protocol's frames: hello, request, response, ops, events |
| `readiness.rs` | `Readiness` | The owner-local readiness answer — never a secret |
| `handshake.rs` | `load_or_create` / `proof` | `daemon.token` (0600, atomic hard-link publish) and the two-way `proof(Side, nonce, token)`; the token never crosses the socket |
| `socket_path.rs` | `plan` / `expected` | `daemon.sock` beside the data, or a private 0700 runtime directory when the path is over 100 bytes; ownership and unsafe-mode checks |
| `runtime_record.rs` | `RuntimeRecord` | `daemon.json`: where the live coordinator bound; discovery only, never identity |
| `client.rs` | `probe` / `call` / `subscribe` | Blocking client: connect, authenticate, and one op or a held event subscription, all under deadlines |

### The coordinator itself

| File | Primary item | Purpose |
| --- | --- | --- |
| `state.rs` | `State` / `auth_view` | Shared readiness snapshot behind a mutex, plus the broadcast event channel every subscriber reads from |
| `worker.rs` | `Queue` / `pass` | The coalescing pass worker thread: every wake since the last pass is one pass (`auto::run_pass` under `sync.lock`), plus a verify when `[sync] verify_every` is due |
| `channel.rs` | `spawn` | The workspace-channel thread: probes the store read-only and follows only once it is ready and a usable, non-suspended credential stands; restarts on reload/login/logout and turns a `hello`/`change` nudge into `queue.wake(Trigger::Channel, …)` |
| `server.rs` | `Ctx` / `serve` | The control server: accept, greet, authenticate, dispatch one `Op` per connection (or hold a subscription open) |
| `coordinator.rs` | `run` | The foreground entry point (`comemory sync daemon run`): acquire `daemon.lock`, bind the socket, spawn the worker/channel/watchdog tasks, and run until SIGTERM or the data directory disappears |
| `watchdog.rs` | `bind` / `guard` / `tick` | Bind over a leftover socket, the reconciliation ticker (re-reads `daemon_interval` every cycle), and the guard that re-binds a removed socket within one tick and stops the coordinator when its data directory is gone |
| `spawn.rs` | `spawn` | `Kind::Process` supervision: starts a fresh coordinator with the hidden run-only `--detach-session` flag, which safely calls `setsid` before coordinator initialization; stdio goes to `logs/sync-daemon.{out,err}.log` for hosts with no usable launchd/systemd |

### Lifecycle and CLI-facing

| File | Primary item | Purpose |
| --- | --- | --- |
| `ensure.rs` | `ensure` / `Intent` | Verify (and, unless a read-only preflight already found one healthy, repair) the coordinator for a data directory. `daemon-ensure.lock` serializes concurrent repairs to one; shutdown requires an authenticated control connection, never an unverified PID from stale metadata |
| `status_view.rs` | `view` / `StatusView` | `comemory sync daemon status`'s live probe report — starts nothing, never calls `ensure` |
| `supervisor.rs` | `Kind` / `detect` / `activate` | Which OS backend keeps a directory's coordinator running (`launchd` / `systemd` / `process` / `external`) and its per-directory unit lifecycle; `remove_legacy` retires the pre-#257 single, un-id'd unit |

Tests live beside the modules under `tests/`. When you add a file here, add
its row above. No `mod.rs` barrel.
