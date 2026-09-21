# Replication end-to-end harness — Design

**Date:** 2026-09-21   **Status:** Approved   **Author:** Auto
**Topic:** Real-process replication harness and coverage gates for issue 249.

Parent epic: Falconiere/comemory#248. This issue ships the harness only.
Production sync behavior stays as it is.

## Problem

Replication crosses the engine CLI, the platform API, and a workerd
Durable Object. A test that preprograms an import response can stay green
while two machines never converge. The engine's `SyncPlatformServer`
(`tests/common/sync_platform_server.rs`) is that canned server. It must
not become the evidence for this epic.

Contributors also have no single command that builds one engine artifact,
boots the real platform stack, and fails when a required case was skipped.

## Non-Goals

1. No production change to sync, the daemon, the journal, or the wire
   protocol. Those are issues 250–258.
2. No extension of `SyncPlatformServer` and no new mock HTTP server.
3. No repository-policy matrix. Issue 183 owns allow, withhold, and revoke.
   This harness provisions two workspaces and syncs only the first.
4. No unattended sub-2s fan-out. The p95 below measures an explicit
   `comemory sync` round on loopback. Lost notifications are a separate
   case that waits for the existing five-second reconciliation.
5. No public-CI checkout of private `CodaSignal/comemory.io`. Live cases
   run in that repo, pinned to an engine revision. The public engine
   workflow runs the cases that do not need the platform tree.

## Architecture

One engine entry point drives both places:

```text
scripts/test-replication-e2e.sh --platform-root PATH --case NAME
  → verify engine binary and platform revision
  → refuse a missing runtime before any assertion
  → bun test apps/api/src/routes/__tests__/replication-harness.ts
     with REPLICATION_CASE=NAME
```

The platform harness reuses the stack already in production tests:

| Piece | Existing module |
| --- | --- |
| Real CLI, isolated `auth.json`, git checkout | `apps/api/src/routes/__tests__/repository-sync-e2e-support.ts` |
| Org, key, workspace seed | `repository-sync-test-support.ts` (`seedRepositoryPolicy`) |
| `comemory serve` engine host | `utilities/__tests__/engine-host-process.ts` |
| HTTP API | `Bun.serve` forwarding to the real `app.fetch` |
| Workerd channel | `workspace-channel-runtime.ts` |

Trade-off: the API process is the real Hono app inside Bun, which is how
`repository-sync-e2e.test.ts` already proves sync. A second workerd copy
of the whole API would not reuse that infrastructure. Workerd is still
booted for the channel, and a missing `node` or wrangler fails the run.

The shell owns process-group teardown. The bun process runs in its own
group. On exit, interrupt, or a 20-minute deadline the script signals the
group, waits 15 seconds, then sends `SIGKILL`. A hung child cannot keep
the job green.

A local TCP proxy sits in front of `Bun.serve` for the fault cases. It
forwards bytes unchanged unless the case asks it to drop the first full
response of one `POST /v1/sync/*` or to flip one body byte. Clients set
`api_url` to the proxy. There is no second implementation of sync.

### Where CI runs

| Repo | Required command | What it proves |
| --- | --- | --- |
| Engine | `bash scripts/check-replication-coverage.sh` | Manifest pairs every G-id with a case and a test file |
| Engine | `--case teardown` and `--case missing-runtime` | Process-group kill; wrong binary fails before assertions |
| Platform | `--platform-root . --case <live case>` with `COMEMORY_POLICY_BIN` | Baseline, faults, credentials, propagation |

`scripts/replication/platform.sha` is the platform commit the harness
requires. The runner reads that pin, and the engine SHA, from the tree
that contains the script, not from the caller's git root. A checkout
whose history does not contain that commit fails.
The run prints the checkout's HEAD, which may be a descendant.
`apps/api/replication-engine.sha` in the platform repo is the engine
commit that CI builds. The harness file is
`apps/api/src/routes/__tests__/replication-harness.ts`. It is not named
`*.test.ts`, so a normal `bun test` does not collect it and then skip it.
A run prints both SHAs, `comemory --version`, and the workerd URL.

## Interfaces / Schema

### Runner

```text
bash scripts/test-replication-e2e.sh \
  --platform-root /path/to/comemory.io \
  --case baseline \
  [--engine-bin /path/to/comemory]
```

`--engine-bin` defaults to a `cargo build` of this tree. The script
exports `COMEMORY_POLICY_BIN` and an empty `COMEMORY_API` /
`COMEMORY_API_KEY`. `HOME` is a fresh temp directory so a developer
`~/.comemory/auth.json` is never read. Exit 0 only when the case's
assertions passed. Exit 2 when a runtime, binary, or pin check failed
before assertions. Any other failure exits 1.

### Coverage manifest

`scripts/replication/coverage.json`:

```json
{
  "version": 1,
  "acs": {
    "G-1": ["baseline"],
    "G-2": ["baseline"],
    "G-3": ["baseline"],
    "G-4": ["coverage"],
    "G-5": ["missing-runtime", "baseline"],
    "G-6": ["teardown", "fault-ack", "corrupt", "credentials"],
    "G-7": ["propagation", "lost-nudge"]
  }
}
```

`scripts/check-replication-coverage.sh` reads that file and fails when:

- an AC key is missing, duplicated, or mapped to an unknown case
- a case name is not claimed by any AC
- a case string is missing from `scripts/test-replication-e2e.sh`
- `--platform-root` is set and the platform harness file does not mention the case
- `--report PATH` is set and that JSON has `tests_ran: 0` or `skipped > 0`

The checker is a gate in `scripts/check-all.sh`. It does not boot the
platform.

### Report

Each live run writes `replication-report.json` next to the temp root
and prints the same object:

```json
{
  "engine_sha": "<40 hex>",
  "platform_sha": "<40 hex>",
  "engine_version": "<comemory --version>",
  "case": "baseline",
  "tests_ran": 1,
  "skipped": 0,
  "workspaces": ["<id>", "<id>"],
  "fixture_commit": "<40 hex>"
}
```

### Cases

| Case | Needs platform | Observable result |
| --- | --- | --- |
| `baseline` | yes | Saved body from `docs/database-decision.md` is in the receiver store and in the engine host. The report's `fixture_commit` is the real `git rev-parse HEAD` of a checkout of `engine-memory-id.ts` |
| `missing-runtime` | no | `/bin/false` or a missing `node` exits 2 and prints `replication: runtime` |
| `teardown` | no | A `sleep 120` grandchild is gone within 15s of the parent exiting |
| `fault-ack` | yes | Dropping one sync acknowledgement still leaves one markdown file, one content hash |
| `corrupt` | yes | One flipped request byte is rejected; the receiver store gains no file |
| `credentials` | yes | A decoy `HOME` auth.json pointing at `127.0.0.1:1` is never contacted |
| `propagation` | yes | 100 sequential saves, each explicit sync, p95 of commit-to-visible ≤ 2s |
| `lost-nudge` | yes | With `comemory sync daemon run` looping, a dropped channel frame is still pulled on the next 5s cycle |
| `coverage` | no | The checker rejects a fixture manifest with a dangling AC |

`baseline` creates two workspaces and two client stores. Both clients
bind to workspace A. The report lists both workspace ids. Workspace B
is not written and is not a policy test.

Body text is the first 1500 characters of the platform's
`docs/database-decision.md`, the same slice `repository-sync-e2e.test.ts`
already saves. The code fixture, when a later case needs one, copies
`apps/api/src/services/engine-memory-id.ts` into a real git repo the way
`indexPolicyCheckout` does. This issue's baseline is the memory save.

## Failure modes and edge cases

| Condition | Behavior |
| --- | --- |
| `--platform-root` missing, not a checkout, or `platform.sha` is not an ancestor of that checkout | Exit 2, message names the path and both SHAs. No test process. |
| `--engine-bin` missing, not executable, or `--version` fails | Exit 2 before `bun`. |
| `node` or the platform's wrangler package missing | Exit 2 naming the missing command. |
| Case name absent from the manifest | Exit 2. The coverage gate fails the same way in `check-all`. |
| Proxy drops one acknowledgement | The next `comemory sync` on the same stores converges. Receiver has one file, one content hash. |
| Proxy flips one body byte | The request is rejected. Receiver memory count stays 0. The run exits 1 if a file appears. |
| API, workerd, or client killed mid-sync | Restart the same temp stores and run `sync` again. The content hash appears once. |
| Child ignores `SIGTERM` or fills a pipe | The script's `SIGKILL` after 15s reaps the group. The teardown case fails if any child remains. |
| Developer cloud credentials in the real `HOME` | Invisible. The case fails if the decoy `127.0.0.1:1` accepts or if the real `HOME` path appears in the command log. |
| Bun reports a skip or zero tests | The runner exits 1. `--report` makes the coverage checker exit 1. |
| Platform repo absent on the public runner | Live cases are not invoked there. The public job does not skip them silently: it does not list them as its command. Platform CI fails if its runner lacks workerd. |

## Acceptance criteria

- **AC-1:** `bash scripts/test-replication-e2e.sh --platform-root <comemory.io> --case baseline` prints both git SHAs and `comemory --version`, creates two workspace ids, and exits 0 only after the receiver's `memories/*.md` contains the saved slice.
- **AC-2:** That receiver file's body equals the slice saved from `docs/database-decision.md`, and the engine host's memory GET returns the same body.
- **AC-3:** The saved bytes equal the slice read from `docs/database-decision.md` at runtime. `baseline` also runs `git init` and `git commit` on a copy of `apps/api/src/services/engine-memory-id.ts` and records that `HEAD` in `fixture_commit`. The harness and runner contain no `SyncPlatformServer` import.
- **AC-4:** `bash scripts/check-replication-coverage.sh` exits 0 on `scripts/replication/coverage.json` and exits 1 on a copy that names an AC with no case or a case the runner does not mention.
- **AC-5:** `--case missing-runtime --engine-bin /bin/false` exits 2 and prints `replication: runtime`. Platform CI runs `--case baseline` against the SHA in `engine.sha`. A missing `node` on that runner exits 2 rather than a skipped test.
- **AC-6:** `--case teardown` leaves no `sleep 120` process 15s after the wrapper exits. `--case fault-ack` drops one sync acknowledgement, runs `comemory sync` again, and leaves one receiver file. `--case corrupt` yields zero receiver files.
- **AC-7:** `--case propagation` runs 100 real saves of distinct slices and records p95 commit-to-visible ≤ 2 seconds, measured from the sender `sync` exit to the receiver file appearing after the receiver `sync`. `--case lost-nudge` starts `comemory sync daemon run`, drops one channel frame, and the receiver file appears on the next daemon cycle, within 5 seconds plus the sync request.

## Acceptance evidence

| AC | Real input | Expected result | Boundary | Check |
| --- | --- | --- | --- | --- |
| AC-1 | Platform checkout at `platform.sha`; engine binary built here; two temp stores | Report lists two workspace ids and both SHAs; exit 0 | Wrong SHA or `/bin/false` exits 2 first | `--case baseline`; `--case missing-runtime` |
| AC-2 | First 1500 chars of `docs/database-decision.md` | Same characters in receiver markdown and engine GET | Empty slice is a harness bug and exits 1 | `--case baseline` |
| AC-3 | `docs/database-decision.md` and `engine-memory-id.ts` on disk | Body bytes match the doc slice; `fixture_commit` is 40 hex; runner and harness have no `SyncPlatformServer` import | A missing doc file exits 1 before sync | `--case baseline`; coverage checker source scan |
| AC-4 | `coverage.json` and a temp copy missing `G-2` | Good file exits 0; bad file exits 1 and names `G-2` | Zero `tests_ran` in a sample report exits 1 | `bash scripts/check-replication-coverage.sh` and `--report` fixture |
| AC-5 | `/bin/false`; platform workflow pin | Exit 2 and `replication: runtime`; platform job builds the pinned SHA | Unset `node` exits 2 | Engine `--case missing-runtime`; platform workflow |
| AC-6 | `sleep 120` grandchild; one dropped `POST /v1/sync` response; one flipped body byte | No sleep pid; one file; zero files | `SIGTERM` ignored, then `SIGKILL` | `--case teardown`; `--case fault-ack`; `--case corrupt` |
| AC-7 | 100 distinct prefixes of the same doc; one suppressed websocket frame while `sync daemon run` is looping | p95 ≤ 2s on explicit sync; file appears on the next 5s daemon cycle | Bulk drain is not this case and has no latency claim | `--case propagation`; `--case lost-nudge` |

## Documentation impact

- This design is the contract.
- `docs/README.md` links it.
- `docs/guides/replication-e2e.md` states prerequisites (`node`, the
  platform checkout, a built `comemory`), the pin files, the case list,
  and which repo's CI runs which case.
- No CLI flag or config key changes, so `docs/cli-reference.md` stays.

## Open Questions

None. The p95 case is an explicit loopback `sync` round because this
issue forbids production changes; unattended fan-out stays in issue 184.
The second workspace is provisioned and reported, and issue 183 owns
isolation assertions. Public engine CI cannot clone the private platform
repo; live cases are required in platform CI instead of a skipped engine
job.
