# `domains/sync/drain/`

The `replica-v1` exchange client (#255): everything one pass of `comemory
sync`, a hook, the daemon or a watch nudge does to drain this machine's
outbox upstream and the upstream's feed here — negotiation, the policy it runs
under, push, pull, holds, replay, verification and workspace keying. Design:
[`docs/designs/2026-09-24-replica-exchange-client.md`](../../../../docs/designs/2026-09-24-replica-exchange-client.md).

**What does NOT belong here:** the engine halves the upstream runs
(`../replica/`), SQL (`src/store/`), and rendering (`src/cli/sync_render.rs`).

## Contents

| File | Primary item | Purpose |
| --- | --- | --- |
| `adopt.rs` | `events` | Queue local feed positions the outbox never saw: events before every push, pre-outbox documents at the upgrade |
| `anchor.rs` | `check` | Whether the upstream still holds, at the cursor's position, the entry the cursor was left on |
| `backoff.rs` | `after_failures` | Full-jitter delays: between in-pass retries and across passes |
| `code_capture.rs` | `capture` | Capture each approved repository's current code index as a generation to upload; activate one on acceptance |
| `holds.rs` | `reconsider` | Rewind the cursor for held pull positions whose cause is gone |
| `keying.rs` | `stamp_outgoing` | Whose data a pending operation is: stamps, the last-used-key rule, foreign bindings |
| `legacy_pass.rs` | `run` | One pass on the old protocol, under the same loop, budget and network state |
| `negotiate.rs` | `decide` | The protocol table: what a stored selection and a manifest answer select |
| `network.rs` | `fail` | The network state a key carries between passes (backoff, suspension, protocol error) |
| `pull.rs` | `page` | One `replica-v1` pull page, resolved entry by entry; the cursor takes the raw continuation |
| `pull_rules.rs` | `classify` | What one pulled entry deserves (rules 1–7), decided before anything is written |
| `push.rs` | `batch` | One push batch in the order made, and what the answer does to each row |
| `push_batch.rs` | `prepare` | Turning an outbox row into the operation a push sends; staged parts for an oversized one |
| `push_hold.rs` | `classify` | Why a pending operation is not sent this pass |
| `rebootstrap.rs` | `begin` | A replaced stream: restart the cursor, drop holds, clear synced positions, start a replay |
| `replay.rs` | `step` | One step of a replay in progress — a scan page or an apply batch — in the phase it stopped in |
| `replay_apply.rs` | `batch` | Phase 2 of a compacting replay: apply each entity's last entry in position order |
| `replay_scan.rs` | `page` | Phase 1 of a compacting replay: keep each entity's last entry |
| `replica_pass.rs` | `run` | One `replica-v1` pass: pull first, then alternate pages and batches to the captured head |
| `report.rs` | `Report` | The `exchange` leg of every sync report |
| `retry_after.rs` | `parse` | `Retry-After` as seconds or an HTTP date, capped at an hour |
| `session.rs` | `open` | Opening a pass (key, gates, policy, protocol) and closing it (network state, row saved) |
| `status.rs` | `status` | The `exchange` block of `comemory sync --action status`, read offline |
| `transport.rs` | `Transport` | One request builder and one answer classification (`Failure`) for every upstream call |
| `upgrade.rs` | `begin` | The first pass after a key selects `replica-v1`: the upgrade horizon and document adoption |
| `verify.rs` | `run` | `comemory sync --action verify` on a `replica-v1` key: per-kind buckets and kind-scoped repair |

Tests live beside the modules under `tests/`. When you add a file here, add its
row above.
