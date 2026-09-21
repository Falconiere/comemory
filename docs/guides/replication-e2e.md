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
| `coverage` | engine | A manifest missing `G-2`, and a report with `tests_ran: 0`, are rejected |
| `teardown` | engine | A child that ignores `SIGTERM` is gone within 15 seconds |
| `missing-runtime` | engine | A binary that cannot print `--version` exits 2 |
| `baseline` | platform | A saved document slice reaches a second store; two workspaces are provisioned |
| `fault-ack` | platform | A dropped sync acknowledgement converges to one file on the next sync |
| `corrupt` | platform | One flipped request byte leaves the receiver empty |
| `credentials` | platform | A decoy `HOME` auth file is never contacted |
| `propagation` | platform | 100 explicit sync rounds, p95 ≤ 2 seconds |
| `lost-nudge` | platform | `sync daemon run` pulls the file on the next 5 second cycle |

`bash scripts/check-replication-coverage.sh` is also a `check-all` gate.
It does not boot the platform. Pass `--platform-root` to also require the
harness file to name every live case, and `--report PATH` to reject a run
that executed or skipped nothing.
