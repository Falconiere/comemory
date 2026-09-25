# Replication end-to-end harness

Run the real engine binary against a real platform checkout. The contract
is [the design](../designs/2026-09-21-replication-e2e-harness.md).

## Prerequisites

- This repository, with Rust stable enough to `cargo build`.
- A checkout of `CodaSignal/comemory.io` whose history contains this
  repo's `scripts/replication/platform.sha` as an ancestor.
- `node` on `PATH`, and `wrangler` installed in that checkout
  (`bun install` at the platform root).
- `bun` on `PATH` for any live case.

The public engine CI job does not clone the private platform repo. It runs
the coverage gate, `--case teardown`, and `--case missing-runtime`. Live
cases run in the platform repository against the engine commit in
`apps/api/replication-engine.sha`.

## Run

```bash
bash scripts/test-replication-e2e.sh --case coverage
bash scripts/test-replication-e2e.sh --case teardown
bash scripts/test-replication-e2e.sh --case exchange
bash scripts/test-replication-e2e.sh \
  --platform-root /path/to/comemory.io \
  --case baseline
```

`--engine-bin` defaults to `target/debug/comemory` after a debug build.
`--case missing-runtime --engine-bin /bin/false` exits 2 and prints
`replication: runtime`.

## Cases

| Case | Where it runs | What it asserts |
| --- | --- | --- |
| `coverage` | engine | A manifest missing `G-2`, and a report with `tests_ran: 0`, are rejected. Feature issues add their own AC family (`F-1`…) mapped to their case |
| `teardown` | engine | A child that ignores `SIGTERM` is gone within 15 seconds |
| `missing-runtime` | engine | A binary that cannot print `--version` exits 2 |
| `contract` | engine | The `replica-v1` journal against real processes: digests over metadata, replay receipts, conflicting bytes, epoch mismatch, envelope caps, staged activation, tombstone ordering, workspace scope, route classes, rebuild identity, and legacy-wire convergence (`--test replica_contract` / `replica_contract_2`) |
| `events` | engine | Feedback verdicts and activity runs as `replica-v1` events (#254): stable event ids and origin devices, one counter contribution per event through replays, echoes, an induced fault and a SIGKILL mid-envelope, the one-time backfill, retention expiry and purge erasure reaching the journal, tied timestamps kept in acceptance order, and the allowlist / path / secret policy on what text may leave (`--test replica_events` / `replica_events_2` / `replica_events_3`; ACs `V-1`…`V-7`) |
| `exchange` | engine | The exchange client (#255) against real `comemory serve` hubs behind a real fault proxy (drop, hold, delay, flip, rate-limit): negotiation and a one-way upgrade, a 2,000+ operation backlog drained in one run, held pages then an allowed entry, stalls that replay safely, held states that never starve the rest, `429`/`502`/timeout survival, restored and replaced streams, workspace switches, code generations and documents both ways, and feedback and activity events counted once (`--test replica_exchange` … `replica_exchange_8`) |
| `baseline` | platform | A saved document slice reaches a second store; two workspaces are provisioned |
| `fault-ack` | platform | A dropped sync acknowledgement converges to one file on the next sync |
| `corrupt` | platform | One flipped request byte leaves the receiver empty |
| `credentials` | platform | A decoy `HOME` auth file is never contacted |
| `propagation` | platform | 100 explicit sync rounds, p95 ≤ 2 seconds |
| `lost-nudge` | platform | `sync daemon run` pulls the file on the next 5 second cycle |

Run the event suite on its own with
`bash scripts/test-replication-e2e.sh --case events`; like `contract` it needs
no platform checkout, so the public engine CI runs it.

`bash scripts/check-replication-coverage.sh` is also a `check-all` gate.
It does not boot the platform. Pass `--platform-root` to also require the
harness file to name every live case, and `--report PATH` to reject a run
that executed or skipped nothing.
