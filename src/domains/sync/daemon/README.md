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

| File | Primary item | Purpose |
| --- | --- | --- |
| `identity.rs` | `BinaryIdentity` | Canonical data directory, its short id, and the binary a coordinator runs |
| `control.rs` | `Op` | The control protocol's frames: hello, request, response, ops, events |
| `readiness.rs` | `Readiness` | The owner-local readiness answer — never a secret |
| `handshake.rs` | `server_proof` / `client_proof` | `daemon.token` (0600) and the two-way proof; the token never crosses the socket |
| `socket_path.rs` | `plan` / `locate` | `daemon.sock` beside the data, or a private 0700 runtime directory when the path is over 100 bytes; ownership checks |
| `runtime_record.rs` | `RuntimeRecord` | `daemon.json`: where the live coordinator bound; discovery only, never identity |
| `client.rs` | `probe` | Blocking client: connect, authenticate, and one op or a subscription, all under deadlines |

Tests live beside the modules under `tests/`. When you add a file here, add
its row above. No `mod.rs` barrel.
