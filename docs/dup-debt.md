# Duplication debt baseline (`scripts/dup-check.sh`)

Status: documented baseline, tracked by a count ratchet against a **pinned**
`similarity-rs` · Owner: whoever burns a pair down next

**101 near-duplicate function/method pairs at threshold 0.85**, measured with
**`similarity-rs 0.5.0`** over the 726 production `.rs` files under `src/`. That
number and the tool that produced it are recorded together, here and in
`dup-baseline.txt`, because either one alone is meaningless. Of those 115,
**101 are earned in `dup-ledger.txt`** (framework-forced / twin-table /
documented-lifecycle) and **0 remain unledgered** — the hard residual budget
(5% of the 285-pair anchor).

**Why this number fell from 107 (reserved-tail burn, `dedup-m2`).**
Macro-hid the last six unledgered pairs (job spawn wrappers, score
normalizers, id CSV parsers, auth loaders, near-dup scan). −6 raw:
107 → 101; unledgered 6 → 0.

**Why this number fell from 115 (same-name + ceiling lock, `dedup-m2`).**
Macro tables burned the reserved unledgered tail: store cursor/upsert/schema
probes, envelope refusal constructors, router auth-gate responses, and the
`respond`/`accepted` + `guard_*` route helpers. −8 raw: 115 → 107;
unledgered 14 → 6.

**Why this number fell from 123 (M3+ cluster burns + same-name macros,
`dedup-m2`).** Macro tables hid same-named constructors and trait methods the
ledger cannot name (`serve::jobs::events` / `retrieval::fuse` /
`replica::validate` / `skill_links` / `memories::list`); `Filters::window` was
inlined to `filters.scope.window()` at four call sites. Store/serve path and
ORM helper macros from the prior M3 commit already cut 206 → 123. −8 raw:
123 → 115. The unledgered count then dropped 83 → 14 by earning 69 more
ledger rows for deliberate twins already narrated below.

**Why this number fell from 263 (M2 serve dedup, `dedup-m2`).** One
`QueryOrJson` extractor collapsed the GET/POST twins on `find`, `code/search`,
`search`, and `memories/search`; shared `mutate_blocking` / `with_ctx` /
`mutate_response` helpers shortened the mutating-handler tail across memories,
trash, repos, sources, config, hooks, maintenance, and the learning console;
and the `id_collision_for_test` / `id_collision_inner` test twin was deleted so
tests call `id_collision` directly. −13 total: 276 → 263 at that step; later
M3 macro burns and ledger accounting brought the live raw count to 115.
Per-file pair counts on every touched route either fell or held; `maint/gc.rs`
stayed hand-written because converting `update_policy` raised that file from 0
to 1 pair.

**Why this number fell from 276 (pre-M2 serve extract, `dedup-m2`).** The M2
serve extractor work landed first (276 → 263); see the paragraph above.

**Why this number fell from 284 (M1 dedup burn, `deduplication-enforce`).**
Five mechanical merges, each verified pair-gone with no new pair and covered
by the modules' existing boundary tests: `tune`/`bandit` share one generic
`spawn_learning_job` plus a macro-generated per-type glue (the hand-written
glue paired at 100%, so the macro follows the `wire_strings!` precedent);
`reembed_memories`/`reembed_code` share `reembed_leg` with the two call blocks
inlined in `run` (thin wrappers re-paired at 92.10%, so no wrappers remain);
`write_list`/`write_rows` share `write_page` over a render closure (TTY bytes
identical); `push_direct_edges`/`push_co_citation_edges` share `push_edge_set`
with a reverse flag; the four `copy_*` wrappers become one `DOCUMENT_TABLES`
const plus a shared `copy_all` looper (also used for `SHARED_DOCUMENT_TABLES`,
so the two loops cannot pair). −8 total: 284 → 276. The 285 → 284 step came
from #408 with no doc entry; this entry restores the chain.

**Why this number rose from 284 (#336, plan proposals).** The MCP transport
gains a second project writer, so `mcp::tools_project_writes::{project_evidence,
project_propose}` pair: the rmcp-forced tool skeleton recorded under
`src/mcp/` below. Both bodies are already one `write_tool(…, enveloped)` call
over a different core's request type, so no shared logic is left to extract.

**Why this number held at 284 (#331, project activity page).**
The activity page adds no pair. Its CLI verb and its route each live in
their own module (`cli::project_activity`, `serve::routes::project_activity`).
Kept in `cli::project` and `serve::routes::project_request`, they paired with
`render_page` and `list_field`, and a shared TTY rows helper tried first made
the count worse. `serve::routes::projects::read` now serves the path-only
reads too (`Ok(request)`), so it and #335's `by_id` no longer pair.

**Why this number fell from 289 (#326, projects foundation).** The four
fixed refusal constructors in `serve::envelope` (`unauthorized`, `busy`,
`read_only`, `confirmation_required`) now share one `refusal` helper, which
burns five of their six pairs; `unauthorized` ↔ `confirmation_required` stays.
The projects capability adds one pair: `mcp::tools_projects::{project_list,
project_show}`, the rmcp-forced tool skeleton every read tool shares. Every
other pair the new store, domain and route code produced was burned down in
the same change (one relation writer over `execute_many`, one relation read,
one `invalid_request` builder with an edge/message modifier).

**Why this number fell from 285 (#335, the plan read).** `project_show` now
picks the charter or the plan core by its `view`, so its body no longer
mirrors `project_list` and the `mcp::tools_projects::{project_list,
project_show}` pair is gone. The plan read added no pair of its own: the four
plan selects share two differently shaped functions in `store::project_plan`
rather than one near-identical function per table, and `GET /projects/{id}`
and `GET /projects/{id}/plan` share one `by_id` handler.

**Why this number rose from 288 (#257, `watch` attaches to the coordinator).**
`cli::watch::{follow_coordinator, attach_once}` pair: `follow_coordinator` is
the retry loop (attempt count, full-jitter backoff, `once` short-circuits on
both `Ok`/`Err`) around one attempt; `attach_once` is that one attempt
(subscribe, report `Connected`, then either one `catch_up` or an event-read
loop). Splitting `attach_once` further — a bare `once`/`forever` function
pair with no shared retry wrapper — was tried and made this *worse*, not
better: the resulting three short functions (each opening with "subscribe,
report `Connected`, then …") scored two pairs against each other instead of
one, since `similarity-rs` compares shape, not the caller's actual retry
semantics. Kept as the two-function form.

**Why this number rose from 287 (#257, hook and save wakes).**
`cli::sync_auto::{run, run_in_process}` pair: both end with "build a JSON
report, or print nothing" once `json_flag` is checked, but `run` sends one
wake and reports the coordinator's instance while `run_in_process` (only
reachable under the `COMEMORY_SYNC_DAEMON=0` harness switch, where there is
no coordinator to wake) runs the whole pass in-process and reports every
leg. Merging them behind an `in_process: bool` would replace two short,
readable functions with one long one branching on it throughout — the same
bodies with a worse name.

**Why this number rose from 285 (#257, the required resident daemon).**
`domains::sync::daemon::supervisor` gives every lifecycle action one function
dispatching on the four-variant `Kind` (launchd/systemd/process/external,
each launchd/systemd arm doing real `launchctl`/`systemctl` work, the other
two a no-op) — the raw increase was **nine** pairs and **seven were burned
down**: `write` and `start` merged into one `activate` (every caller ran them
back to back — issuing two native round trips per backend was never a real
choice), `remove_legacy` stopped hand-rolling its own `launchctl`/`systemctl`
calls and now builds a `Unit` for the legacy label through a new
`legacy_unit` helper and calls the existing `remove`, and `cli::sync_daemon`'s
`stop`/`uninstall` share one `report(json_flag, key, message)` for their
`--json`-or-one-line-TTY tail.

Two are inherited: `supervisor::stop` (terminate the running job, leaving the
unit file) recorded against `remove` (which calls `stop` first, then deletes
the file) and against `legacy_unit` (builds the pre-#257 label's `Unit`, no
process involved). They cannot merge: `Intent::Restart` needs "stop, keep the
definition," `uninstall` needs "stop and remove," and a legacy unit's path
never touches a running process — collapsing any pair would need an
`if remove_file` flag or a mixed action/lookup function, which is the same
four-arm `match Kind` bodies with a worse name and a parameter that lies
about what one call site or the other actually wants.

**Why this number fell from 286 (hook-driven auto-sync).** The sync pass
lock added ten raw pairs and all ten were burned before recording: the two
lock-file names left `config::Paths`, where each one-line join paired with
every sibling getter, for `domains::sync::auto::{PASS_LOCK, QUEUE_LOCK}` and one
`hold_pass_lock` helper; `FileLock::acquire` and `try_acquire` share one
`open`. Taking that helper at the top of `run_all` and `push_only` also moved
`run_all` far enough from `push_only` to drop their recorded pair below.

**Why this number rose from 285 (#253).** Document replication added the pulled
cache, its search half and the acceptance path. The raw increase was **eleven**
pairs and **ten were burned down**, so exactly one is inherited:

- `retrieval/doc_route.rs` (1 inherited). `local_hit` and `shared_hit` resolve
  one search hit into the same `Resolved`, and report against each other because
  both are "look the rows up, or give up". They cannot be one implementation:
  one reads `documents`/`document_chunks`, the other `remote_document`/
  `remote_document_chunk`, and keeping those two table sets apart is the whole
  premise of the feature — an import may not touch a local row. A single
  resolver would have to branch on the side internally, which is the same two
  bodies with a worse name.

The ten burned, for the next reader: `store/remote_document.rs` gave up six —
the header insert moved into the one function that writes it, a passage and its
searchable row are written together, `purge` became a loop over the four tables
that share one key, and the two child readers became one `all::<T>` over a
`PulledRow` trait. `sync/replica/validate.rs` gave up three, including the pair
this file previously recorded as irreducible (see below): adding a third kind
made the decode-and-refuse skeleton worth extracting, so each kind now states
only its invariant through an `Identity` trait. `store/remote_document_view.rs`
gave up one by reading a pulled passage through the typed reader that already
existed rather than a second query of its own.

**Why this number rose from 275 (#252).** Code-generation replication added
four declared tables and the store modules that read and write them, which is
the single largest grandfathered class here — per-table row mappers. The raw
increase was **fifteen** pairs; **five were burned down before recording the
rest**, and only the ten below are inherited:

- `domains/code/generation.rs` (3 burned). `symbols_of` / `imports_of` /
  `co_changes_of` had been split out purely to keep `project` under the
  50-line ceiling, which made three same-shaped map-to-wire helpers. The symbol
  walk moved back into `project` where the manifest it belongs to is built, and
  the two edge sources became one `edges_of`, since "every edge the local index
  can state" is one idea.
- `store/rebuild_copy_code.rs` (3 burned). Every copy here was
  `if old_table_exists(…) { execute_batch(…) }`; the guard is now stated once
  in `copy_if_present`, and `copy_code_generations` no longer reports against
  any of its neighbours. #256 gave `copy_code_generations` a second
  `copy_if_present` call and a pending-outbox probe to keep an owed staged
  generation, which paired it with `copy_code_index_tables`'s own leftover
  `if old_table_exists(…) { … execute_batch(…) }` guard — the one case in this
  file `copy_if_present` had not reached, since its two probed columns had to
  run before the guard could exist. `old_column_exists` already tolerates a
  missing table (an empty `pragma_table_info` scan, not an error), so the
  guard was redundant with the one inside `copy_if_present`; dropping it also
  stopped `copy_code_index_tables` reporting against `copy_code_markers`.
- `sync/replica/validate.rs` (1 burned, 1 inherited — **the inherited pair was
  burned in #253**). `payload_shape` and `code_payload_shape` ran the same three
  checks — payload-vs-tombstone, digest-covers-bytes, decode — before diverging,
  so they collapsed into one `shape` taking the kind's identity rule. The two
  identity rules were then recorded here as irreducible. That held while there
  were two of them; a third kind made the decode-and-refuse skeleton worth
  extracting after all, so they are now one generic `identity::<T>` over an
  `Identity` trait and each kind states only its own contract.
- `store/remote_code.rs` (1 burned of 6, 5 inherited). `files` / `symbols` /
  `edges` became one `projection` reader — a generation is written and read as
  a unit, so three readers that could be called out of step with each other
  were wrong independently of this count. What remains is irreducible: three
  `read` row-mappers, one per projection row type, and `projection` reporting
  against `replace_generation` and `purge_generation`, which are the select,
  insert and delete halves of the same three tables.
- `store/code_generation.rs` (3 inherited). `active`, `active_local` and `all`
  are 3-to-10-line accessors over one table returning `Option` and `Vec`.
  `active_local` could be inlined as `active(…).filter(…)` at its one call
  site, which would clear two of the three — it is kept because "the active
  generation this machine built" is the invariant that prevents a replication
  loop, and spreading it into the caller to satisfy a similarity score would
  trade a named contract for a number.

None of the ten is a case where one implementation could serve both callers.
A raise is not the ratchet's normal direction: it is recorded here rather than
left silent so the next burn-down knows exactly which rows it inherited.

**Why this number rose from 266.** The activity feed added per-table query
projections and short route adapters — the two largest grandfathered classes
below — and the 300-line ceiling split `config/validate.rs`'s per-knob bound
checks into `config/validate_knobs.rs`, which re-reported two pairs under the
new path and surfaced three more among functions that had not moved at all.
Every one of the nine is enumerated below with its rationale; none is a case
where one implementation could serve both callers. A raise is not the ratchet's
normal direction: it is recorded here rather than left silent so the next
burn-down knows exactly which rows it inherited.

## History: why the previous baseline had no authority

`scripts/dup-check.sh` was silently non-functional before Phase 6 of the
toolu-conventions migration: it invoked `similarity-rs` with CLI flags the
installed version doesn't accept and grepped for output shapes the tool never
produces, so it always fell through to a green no-op. Phase 6 fixed the
invocation and recorded **113** pairs. That 113 then stood until issue #200,
and four independent measurements showed it could not be trusted:

1. **It did not reproduce on its own tree.** Re-running `similarity-rs 0.5.0`
   against the tree at `16286113` — the commit that wrote `113` into
   `dup-baseline.txt` — reports **140** pairs over that tree's 226 production
   `.rs` files (its 45 `scripts/*.sh` were passed and, as always, ignored). The
   baseline had been taken with a different `similarity-rs` build, and the
   script pinned no version, so every comparison it fed was apples-to-oranges.
2. **Nothing ran the gate.** `dup-check.sh` appeared in neither
   `scripts/check-all.sh`'s `GATES` array nor any file under
   `.github/workflows/`. Its only caller was the `justfile`'s opt-in `qa`
   recipe. A ratchet nobody runs, over a tool nobody pins, is how the gap went
   unnoticed across sixty merged pull requests.
3. **It skipped silently when the tool was absent**, logging a dim info line
   and exiting 0 — a guardrail reporting success while checking nothing.
4. **This document contradicted itself.** It was headed "The 113 pairs" while
   its tables enumerated only 105 rows.

The #164 domain migration deliberately left all of this unfixed: re-baselining
to make the gate pass would have recorded a number with no more authority than
the one it replaced. Issue #200 pinned the tool, wired the gate into CI, and
re-baselined in one commit, which is what gives the number below its authority.
For the record, the migration itself added **no** duplication: 242 pairs on both
sides of every slice, with an empty normalized pair-set diff.

## What this baseline is measured with

- **Tool:** `similarity-rs 0.5.0`, pinned by the `SIMILARITY_RS_VERSION`
  constant in `scripts/dup-check.sh`. The gate **hard-fails** when a different
  version is on `PATH`, locally as well as in CI: a count from another build
  cannot be compared to `dup-baseline.txt` at all, so failing loudly beats
  passing wrongly.
- **Where CI gets it:** `.github/workflows/test.yml` *derives* the version from
  that same constant and `cargo install`s exactly it, so the workflow and the
  gate cannot drift. Bumping the constant is the only edit a version bump needs.
- **Threshold:** `0.85`, via `--threshold 0.85 --fail-on-duplicates`.
- **Scope:** every tracked `src/**/*.rs` whose path does not contain `/tests/`
  — 724 files (414 at #216; the tree grew). Colocated tests are excluded
  because CORE excludes tests from copy-paste detection: repeated setup there
  is not the duplication worth chasing.
- **Result when #200 fixed the gate:** 242 pairs over the then-current 411
  files. The count was deterministic: three consecutive runs of the pinned build returned 242 every
  time, as did runs with the file list shuffled and reversed, so the number does
  not depend on the order `git ls-files` returns.
- **Shell scripts are NOT covered — a limitation, not a clean result.**
  `similarity-rs` parses Rust and nothing else. Handed the 53 tracked
  `scripts/*.sh` paths alongside the Rust files it silently discarded them and
  reported `Checking 411 files`; run over those 53 alone it prints `No Rust
  files found in the specified paths.` and exits 0 even at threshold `0.5`.
  Earlier versions of this document and of the script claimed `src/ + scripts/`
  coverage, which was never true. The shell paths are therefore no longer passed
  at all: an inert path inflates the target count and would let the gate's
  anti-vacuous guard pass over files nobody analyzes. Duplication in `scripts/`
  is currently **unguarded**; catching it needs a shell-capable detector.
- **Current measurement:** 263 pairs over 724 files after the M2 serve dedup
  (extractor + mutating helpers + test twin, −13) on top of the M1 burn
  (284 → 276) and #408's undocumented 285 → 284 step. Neither the threshold
  nor scan scope changed; the table below was re-derived from the live dump.
- **Every pair is intra-file.** No pair spans two files, so the rationale below
  is recorded once per file rather than once per pair.

Re-run the gate, or the raw scan underneath it, yourself:

```bash
bash scripts/dup-check.sh     # the gate: pinned-version check + ratchet
just qa                       # check-all + deny + dup + machete

# the raw scan the gate wraps
similarity-rs --threshold 0.85 --fail-on-duplicates \
  $(git ls-files 'src/*.rs' | grep -v '/tests/')
```

## How the gate treats this

`similarity-rs` has no native per-pair suppression flag (confirmed via
`similarity-rs --help` — there is `--exclude` for directories and
`--filter-function` / `--filter-function-body` for substring filters, nothing
addressing an individual reported pair). A per-pair allowlist would therefore
mean reimplementing pair identity matching by hand, which is worse than the
problem it solves. `scripts/dup-check.sh` runs a **count ratchet plus a hard
unledgered budget ceiling**, with an earned residual ledger:

- The baseline count (`101`) lives in `dup-baseline.txt`, a single tracked
  integer at the repo root — the same convention as `coverage-floor.txt` and
  `store-leak-baseline.txt`. Exceeding it fails; coming in under it passes
  with a nudge to lower the file.
- `dup-ledger.txt` holds earned residuals as
  `path|fnA|fnB|category|evidence` (fn names sorted, no line numbers, no
  same-name pairs). Categories in use: `framework-forced`, `twin-table`,
  `documented-lifecycle`. Stale ledger rows (not in the live scan) fail.
- The **budget ceiling is 14 unledgered pairs**: 5% of the anchored original
  baseline of **285** (`DUP_BUDGET_ANCHOR` / `DUP_BUDGET_PCT_MAX` /
  `DUP_BUDGET_CEILING`). Unledgered = live total − ledgered. The percentage
  is always `unledgered/285`, never a live function-population rate.
- Every run prints
  `unledgered N/14 (x% of anchor 285) + ledgered M (total T)`.
- **Unledgered > 14 → the gate fails**, even when the raw total is within
  `dup-baseline.txt`. That is the no-duplication rule's hard stop.
- **Current count > baseline → the gate fails.** New duplication introduced by
  a future PR is caught.
- Burning a pair down lowers `dup-baseline.txt` (and drops the matching
  ledger row if it was ledgered) in the same PR. The ratchet only ever
  tightens; the ceiling never moves without changing the anchor constants.

**A ceiling, not an exact match — deliberately one-sided.**
`scripts/store-chokepoint-check.sh` is the repo's *two-sided* ratchet, and its
header draws the contrast on purpose. That gate drives a counted debt to zero
under an active burn-down, so it must fail in both directions to force every PR
to record its own reduction. Duplication has no zero target — the largest
clusters here are documented deliberate per-language and per-table repetition —
and no burn-down campaign behind it, so failing a PR for refactoring unrelated
code *downward* would be friction with nothing to justify it.

**Where it runs, and why not in `check-all.sh`.** The gate is deliberately
absent from `scripts/check-all.sh`'s `GATES` array. It needs a separately
installed tool, and `check-all.sh` is what contributors run on every iteration
and what the release path runs; putting it there would mean either hard-failing
every contributor who lacks `similarity-rs` or skipping silently, and a
guardrail that reports success while checking nothing is the failure mode this
work exists to remove. It is grouped instead with the other two tool-dependent
gates — `deny-check.sh` and `machete-check.sh` — in the `justfile`'s `qa`
recipe, and runs authoritatively as its own step in `.github/workflows/test.yml`,
where the pinned version is guaranteed. Since #205 those other two are pinned
and wired the same way, each by its own constant, so all three are enforced
rather than advisory. When `similarity-rs` is missing, the
gate **fails in CI** and **skips locally with a loud warning** that names the
pinned install command and states plainly that it did not run.

## The 263 pairs by area

Re-derived from the pinned scan (`similarity-rs 0.5.0`, threshold `0.85`): each row below is a pair the
scan reports today, under its own directory, and the rationale column carries the distinction recorded
for it. The ratchet itself re-scans and does not read this table; the counts here are checked against
`dup-baseline.txt` whenever the table is regenerated.

Counts sum to the recorded baseline; if they ever stop matching, one of the two
is stale.

| Area | Pairs |
| --- | --- |
| `src/store/` | 110 |
| `src/mcp/` | 32 |
| `src/config/` | 30 |
| `src/serve/routes/` | 20 |
| `src/domains/sync/` | 8 |
| `src/domains/retrieval/` | 8 |
| `src/serve/jobs/` | 7 |
| `src/domains/integrations/setup/` | 6 |
| `src/serve/routes/memories/` | 5 |
| `src/domains/graph/` | 5 |
| `src/utilities/` | 5 |
| `src/serve/` | 4 |
| `src/cli/` | 3 |
| `src/domains/maintenance/doctor/` | 3 |
| `src/domains/memories/` | 2 |
| `src/domains/code/ast/` | 2 |
| `src/domains/sync/memory_store/` | 2 |
| `src/` | 1 |
| `src/cli/output/` | 1 |
| `src/serve/routes/maint/` | 1 |
| `src/domains/code/` | 1 |
| `src/domains/documents/document/` | 1 |
| `src/domains/learning/` | 1 |
| `src/domains/retrieval/unified/` | 1 |
| `src/domains/integrations/install/` | 1 |
| `src/domains/sync/daemon/` | 1 |
| `src/domains/sync/drain/` | 1 |
| `src/domains/sync/replica/` | 1 |
| **Total** | **263** |

## The 263 pairs, grouped by area

Grouped by directory. Similarity is the pinned detector's score at threshold
0.85; line ranges are function/method spans at scan time. The ratchet re-scans
fresh and does not depend on these line numbers.

### `src/store/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/store/fts_memory.rs:40-58` function `search_memory` | `src/store/fts_memory.rs:82-100` function `search_memory_subtokens` | 98.78% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/fts_memory.rs:40-58` function `search_memory` | `src/store/fts_memory.rs:63-73` function `search_memory_relaxed` | 98.09% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/sync_log.rs:39-46` method `parse` | `src/store/sync_log.rs:69-75` method `parse` | 97.95% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/remote_code.rs:34-39` method `read` | `src/store/remote_code.rs:71-80` method `read` | 97.65% | parallel ORM reads over twin remote-code tables; each keeps its own projection |
| `src/store/remote_code.rs:71-80` method `read` | `src/store/remote_code.rs:109-117` method `read` | 97.65% | parallel ORM reads over twin remote-code tables; each keeps its own projection |
| `src/store/remote_code.rs:34-39` method `read` | `src/store/remote_code.rs:109-117` method `read` | 97.36% | parallel ORM reads over twin remote-code tables; each keeps its own projection |
| `src/store/fts_memory.rs:63-73` function `search_memory_relaxed` | `src/store/fts_memory.rs:82-100` function `search_memory_subtokens` | 97.35% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/sync_log.rs:149-151` function `entries_since` | `src/store/sync_log.rs:154-156` function `local_entries_since` | 96.76% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/sync_log.rs:30-36` method `as_str` | `src/store/sync_log.rs:61-66` method `as_str` | 96.75% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/edges.rs:258-261` function `delete_outgoing` | `src/store/edges.rs:265-273` function `delete_touching` | 94.67% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/sync_state.rs:52-62` function `set_pushed` | `src/store/sync_state.rs:65-75` function `set_pulled` | 94.57% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/replica_outbox.rs:191-193` function `has_pending_for` | `src/store/replica_outbox.rs:202-204` function `has_any_for` | 94.19% | both are one-line callers of the shared `count_for` helper, differing only in the `Option<&str>` state filter they pass — the shared logic is already extracted; a tool comparing two near-identical call expressions cannot see that |
| `src/store/vector.rs:58-60` function `insert_memory` | `src/store/vector.rs:69-71` function `replace_memory` | 94.02% | deliberate per-table repetition across the parallel vector tables |
| `src/store/vector.rs:164-166` function `insert_code` | `src/store/vector.rs:170-172` function `replace_code` | 94.00% | deliberate per-table repetition across the parallel vector tables |
| `src/store/repository_approval.rs:83-96` function `canonical_for` | `src/store/repository_approval.rs:104-114` function `label_for` | 93.95% | lookup by canonical repo id vs by display label; same table, different key |
| `src/store/sources.rs:123-133` function `row_from_sql` | `src/store/sources.rs:343-357` function `file_row_from_sql` | 93.26% | parallel source-row vs. file-row accessors over twin tables |
| `src/store/documents.rs:86-103` function `get_document` | `src/store/documents.rs:211-219` function `get_document_path` | 92.85% | parallel document fetch helpers over twin lookups |
| `src/store/memory_meta.rs:95-112` function `attach_tags` | `src/store/memory_meta.rs:118-146` function `attach_references` | 92.72% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/sync_manifest.rs:24-34` function `live_content_hashes` | `src/store/sync_manifest.rs:38-53` function `live_repository_hashes` | 92.68% | memory content-hash set vs repository-hash set for the same manifest pass |
| `src/store/sources.rs:111-121` function `select_roots` | `src/store/sources.rs:327-341` function `select_files` | 92.64% | parallel source-row vs. file-row accessors over twin tables |
| `src/store/orm.rs:54-60` function `query_one` | `src/store/orm.rs:63-69` function `query_optional` | 92.27% | shared preparation and binding with distinct collection, required-row and optional-row contracts |
| `src/store/eval_runs.rs:218-220` function `set_applied` | `src/store/eval_runs.rs:224-226` function `set_discarded` | 92.20% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/edges.rs:350-371` function `memory_direct_relation_edges` | `src/store/edges.rs:376-390` function `memory_co_citation_edges` | 92.17% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/orm.rs:41-51` function `query_all` | `src/store/orm.rs:54-60` function `query_one` | 92.01% | shared preparation and binding with distinct collection, required-row and optional-row contracts |
| `src/store/repo_marker.rs:75-77` function `last_mined_commit` | `src/store/repo_marker.rs:94-96` function `last_head` | 92.01% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/orm.rs:41-51` function `query_all` | `src/store/orm.rs:63-69` function `query_optional` | 91.73% | shared preparation and binding with distinct collection, required-row and optional-row contracts |
| `src/store/vector.rs:48-50` function `dim_memory` | `src/store/vector.rs:53-55` function `dim_code` | 91.14% | deliberate per-table repetition across the parallel vector tables |
| `src/store/code_ref.rs:46-59` function `materialize` | `src/store/code_ref.rs:85-104` function `upsert` | 90.84% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/memory_row.rs:210-229` function `mined_edges` | `src/store/memory_row.rs:235-252` function `restore_mined_edges` | 90.75% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/code_feedback.rs:49-61` function `own_identity` | `src/store/code_feedback.rs:65-74` function `parent_identity` | 90.72% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/sources.rs:64-77` function `upsert` | `src/store/sources.rs:206-224` function `upsert_file` | 90.50% | parallel source-row vs. file-row accessors over twin tables |
| `src/store/code_generation.rs:198-204` function `active` | `src/store/code_generation.rs:222-231` function `all` | 90.42% | filter to the live generation vs list every generation row |
| `src/store/documents.rs:86-103` function `get_document` | `src/store/documents.rs:179-204` function `get_chunk` | 90.40% | parallel document fetch helpers over twin lookups |
| `src/store/memory_meta.rs:199-225` function `fetch_extra` | `src/store/memory_meta.rs:252-280` function `rank_signals` | 90.07% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/sources.rs:97-103` function `get` | `src/store/sources.rs:265-271` function `get_file` | 90.05% | parallel source-row vs. file-row accessors over twin tables |
| `src/store/replica_pull_hold.rs:42-67` function `record` | `src/store/replica_pull_hold.rs:84-96` function `list` | 89.75% | list held pulls vs record a new hold; shared table, opposite direction |
| `src/store/code_row.rs:325-348` function `find_by_address` | `src/store/code_row.rs:353-368` function `symbol_row_exists` | 89.68% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/sync_policy_snapshot.rs:37-63` function `load` | `src/store/sync_policy_snapshot.rs:69-90` function `save` | 89.59% | read the snapshot vs write it; shared blob, opposite direction |
| `src/store/repo_marker.rs:94-96` function `last_head` | `src/store/repo_marker.rs:120-122` function `root_path` | 89.51% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/retrieval_log.rs:262-273` function `prefix_matches` | `src/store/retrieval_log.rs:278-296` function `distinct_prefix_matches` | 89.37% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/fts_memory.rs:40-58` function `search_memory` | `src/store/fts_memory.rs:106-120` function `search_memory_expanded` | 89.24% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/remote_code.rs:136-184` function `replace_generation` | `src/store/remote_code.rs:223-258` function `projection` | 89.24% | shared SELECT projection vs the replace writer that rebuilds a generation |
| `src/store/indexed_files.rs:22-28` function `delete_for_repo` | `src/store/indexed_files.rs:48-58` function `list_for_repo` | 89.12% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/documents.rs:86-103` function `get_document` | `src/store/documents.rs:111-114` function `delete_document` | 89.03% | parallel document fetch helpers over twin lookups |
| `src/store/fts_memory.rs:63-73` function `search_memory_relaxed` | `src/store/fts_memory.rs:106-120` function `search_memory_expanded` | 89.02% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/remote_code.rs:190-213` function `purge_generation` | `src/store/remote_code.rs:223-258` function `projection` | 88.84% | shared SELECT projection vs the purge writer that drops a generation |
| `src/store/code_feedback.rs:79-89` function `upsert_used` | `src/store/code_feedback.rs:95-104` function `upsert_irrelevant` | 88.81% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/orm.rs:41-51` function `query_all` | `src/store/orm.rs:71-79` function `query_row` | 88.77% | shared preparation and binding with distinct collection, required-row and optional-row contracts |
| `src/store/code_ref.rs:144-165` function `for_rel_live` | `src/store/code_ref.rs:168-195` function `for_memory` | 88.71% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/documents.rs:263-276` function `document_id_in_source` | `src/store/documents.rs:281-294` function `document_ids_for_repo_path` | 88.71% | parallel document fetch helpers over twin lookups |
| `src/store/orm.rs:54-60` function `query_one` | `src/store/orm.rs:71-79` function `query_row` | 88.61% | shared preparation and binding with distinct collection, required-row and optional-row contracts |
| `src/store/fts_memory.rs:82-100` function `search_memory_subtokens` | `src/store/fts_memory.rs:106-120` function `search_memory_expanded` | 88.50% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/documents.rs:211-219` function `get_document_path` | `src/store/documents.rs:250-258` function `document_ids_for_source` | 88.43% | parallel document fetch helpers over twin lookups |
| `src/store/edges.rs:396-412` function `memory_ids_referencing_file` | `src/store/edges.rs:417-435` function `src_ids_for_dst_ids` | 88.41% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/feedback.rs:27-36` function `upsert_used` | `src/store/feedback.rs:41-49` function `upsert_irrelevant` | 88.18% | parallel used-vs-irrelevant counters over twin feedback tables |
| `src/store/memory_meta.rs:152-161` function `ids_matching_kind` | `src/store/memory_meta.rs:302-330` function `keeper_stats` | 87.91% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/repo_marker.rs:75-77` function `last_mined_commit` | `src/store/repo_marker.rs:120-122` function `root_path` | 87.85% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/prune_apply.rs:39-49` function `count_orphan_memory_edges` | `src/store/prune_apply.rs:100-108` function `delete_orphan_memory_edges` | 87.84% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/memory_meta.rs:67-91` function `fetch_rows` | `src/store/memory_meta.rs:118-146` function `attach_references` | 87.58% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/code_row.rs:166-173` function `upsert_repo_root` | `src/store/code_row.rs:183-193` function `upsert_last_indexed` | 87.49% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/repo_marker.rs:126-134` function `exists` | `src/store/repo_marker.rs:138-146` function `set_archived` | 87.45% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/code_row.rs:147-157` function `upsert_indexed_file` | `src/store/code_row.rs:183-193` function `upsert_last_indexed` | 87.33% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/indexed_files.rs:22-28` function `delete_for_repo` | `src/store/indexed_files.rs:62-71` function `delete_one` | 87.32% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/orm.rs:63-69` function `query_optional` | `src/store/orm.rs:71-79` function `query_row` | 87.27% | shared preparation and binding with distinct collection, required-row and optional-row contracts |
| `src/store/repo_marker.rs:101-103` function `archived` | `src/store/repo_marker.rs:126-134` function `exists` | 87.19% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/repo_marker.rs:94-96` function `last_head` | `src/store/repo_marker.rs:126-134` function `exists` | 86.97% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/repo_marker.rs:120-122` function `root_path` | `src/store/repo_marker.rs:126-134` function `exists` | 86.97% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/sources.rs:230-247` function `touch_file` | `src/store/sources.rs:252-262` function `mark_deleted` | 86.96% | parallel source-row vs. file-row accessors over twin tables |
| `src/store/sources.rs:81-84` function `delete` | `src/store/sources.rs:252-262` function `mark_deleted` | 86.92% | parallel source-row vs. file-row accessors over twin tables |
| `src/store/eval_runs.rs:182-190` function `get` | `src/store/eval_runs.rs:218-220` function `set_applied` | 86.85% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/eval_runs.rs:182-190` function `get` | `src/store/eval_runs.rs:224-226` function `set_discarded` | 86.85% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/indexed_files.rs:33-43` function `blob_oid_for` | `src/store/indexed_files.rs:62-71` function `delete_one` | 86.82% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/activity_rollups.rs:64-76` function `distinct_commands` | `src/store/activity_rollups.rs:90-104` function `sample_durations` | 86.81% | two projections over the same filtered window; the shared half is already `activity::apply_filter` |
| `src/store/fts_memory.rs:106-120` function `search_memory_expanded` | `src/store/fts_memory.rs:135-184` function `run_memory_match` | 86.74% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/edges.rs:206-221` function `outgoing` | `src/store/edges.rs:417-435` function `src_ids_for_dst_ids` | 86.72% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/seed_scan.rs:22-35` function `live_memories_after` | `src/store/seed_scan.rs:54-76` function `unshared_documents_after` | 86.72% | memory seed page vs unshared-document seed page; parallel cursors over twin tables |
| `src/store/rebuild_copy.rs:74-81` function `old_table_exists` | `src/store/rebuild_copy.rs:86-93` function `old_column_exists` | 86.68% | one `copy_*` helper per table group in the rebuild copy step, enumerated per group by design rather than generalized |
| `src/store/code_graph_nodes.rs:243-281` function `top_symbols` | `src/store/code_graph_nodes.rs:295-311` function `citing_memories` | 86.65% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/documents.rs:86-103` function `get_document` | `src/store/documents.rs:250-258` function `document_ids_for_source` | 86.54% | parallel document fetch helpers over twin lookups |
| `src/store/bandit_arms.rs:37-56` function `seed` | `src/store/bandit_arms.rs:72-88` function `load` | 86.53% | distinct seed writes and arm-state reads; statement execution is shared by the ORM bridge |
| `src/store/code_sync.rs:140-151` function `cursor` | `src/store/code_sync.rs:154-157` function `set_cursor` | 86.44% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/feedback.rs:110-124` function `used_query_ids` | `src/store/feedback.rs:151-179` function `used_events_for_golden` | 86.42% | parallel used-vs-irrelevant counters over twin feedback tables |
| `src/store/gc_runs.rs:39-54` function `insert` | `src/store/gc_runs.rs:92-121` function `newest` | 86.39% | write a run row vs read the newest; shared column set, opposite direction |
| `src/store/repo_marker.rs:75-77` function `last_mined_commit` | `src/store/repo_marker.rs:126-134` function `exists` | 86.20% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/replica_replay.rs:38-60` function `offer` | `src/store/replica_replay.rs:66-76` function `next` | 86.19% | advance the replay cursor vs offer the next envelope; shared row, different verbs |
| `src/store/memory_meta.rs:67-91` function `fetch_rows` | `src/store/memory_meta.rs:95-112` function `attach_tags` | 86.11% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/documents.rs:111-114` function `delete_document` | `src/store/documents.rs:211-219` function `get_document_path` | 86.07% | parallel document fetch helpers over twin lookups |
| `src/store/memory_meta.rs:53-62` function `fetch_meta` | `src/store/memory_meta.rs:67-91` function `fetch_rows` | 86.05% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/edges.rs:206-221` function `outgoing` | `src/store/edges.rs:258-261` function `delete_outgoing` | 85.97% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/edges.rs:350-371` function `memory_direct_relation_edges` | `src/store/edges.rs:396-412` function `memory_ids_referencing_file` | 85.91% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/indexed_files.rs:33-43` function `blob_oid_for` | `src/store/indexed_files.rs:48-58` function `list_for_repo` | 85.88% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/rebuild_copy_code.rs:145-160` function `copy_mined_edges` | `src/store/rebuild_copy_code.rs:213-226` function `copy_code_virtual_tables` | 85.71% | one `copy_*` helper per table group in the rebuild copy step, enumerated per group by design rather than generalized |
| `src/store/code_row.rs:261-271` function `parent_snippets` | `src/store/code_row.rs:373-382` function `parent_identity` | 85.69% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/doctor_probes.rs:23-32` function `live_memory_hashes` | `src/store/doctor_probes.rs:48-51` function `live_memory_count` | 85.63% | distinct live-memory hash and count projections over the shared live scope |
| `src/store/code_row.rs:122-141` function `purge_file_symbols` | `src/store/code_row.rs:147-157` function `upsert_indexed_file` | 85.60% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/replica_binding.rs:39-66` function `upsert` | `src/store/replica_binding.rs:134-163` function `select` | 85.59% | read the binding vs write it; shared column set, opposite direction |
| `src/store/edges.rs:144-153` function `insert_at` | `src/store/edges.rs:163-172` function `insert_weighted` | 85.55% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/sources.rs:265-271` function `get_file` | `src/store/sources.rs:278-287` function `list_files_by_source` | 85.45% | parallel source-row vs. file-row accessors over twin tables |
| `src/store/index_runs.rs:168-170` function `clamp` | `src/store/index_runs.rs:174-176` function `unsigned` | 85.44% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/query_expansions.rs:35-46` function `insert` | `src/store/query_expansions.rs:93-99` function `count` | 85.44% | distinct expansion insertion and count projection; statement execution is shared by the ORM bridge |
| `src/store/documents.rs:211-219` function `get_document_path` | `src/store/documents.rs:263-276` function `document_id_in_source` | 85.43% | parallel document fetch helpers over twin lookups |
| `src/store/code_graph_nodes.rs:215-219` function `fetch_node` | `src/store/code_graph_nodes.rs:295-311` function `citing_memories` | 85.42% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/repo_marker.rs:53-71` function `read_for_lazy_reindex` | `src/store/repo_marker.rs:138-146` function `set_archived` | 85.40% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/prune_apply.rs:146-175` function `drop_dangling_edges` | `src/store/prune_apply.rs:183-195` function `drop_orphan_code_refs` | 85.35% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/repo_marker.rs:101-103` function `archived` | `src/store/repo_marker.rs:138-146` function `set_archived` | 85.23% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/memory_signals.rs:13-23` function `update_rank_scores` | `src/store/memory_signals.rs:29-45` function `bump_access` | 85.18% | rank batch updates and access-counter updates have different values and write semantics |
| `src/store/edges.rs:322-330` function `delete_co_changed_for_repo` | `src/store/edges.rs:335-344` function `delete_imports_from` | 85.17% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |
| `src/store/activity_rollups.rs:43-61` function `rollups` | `src/store/activity_rollups.rs:90-104` function `sample_durations` | 85.14% | the aggregate and the sample it draws from: different columns, different ordering, different decode |
| `src/store/code_row.rs:78-80` function `stamp_repo_format` | `src/store/code_row.rs:386-394` function `count_for_repo` | 85.08% | parallel code-index row mappers; each retains its own typed projection rather than taking a dynamic table name |
| `src/store/memory_row.rs:366-372` function `live_ids` | `src/store/memory_row.rs:376-382` function `live_bodies` | 85.04% | parallel CRUD/row-mapper pairs over twin tables; each retains its own query and row-decoding contract within `store/` |

### `src/mcp/`

The twelve read tools are one `#[tool_router]` block, and rmcp 3.4 forces the
shape the detector scores: every tool must be a real method carrying
`#[tool(description = "<literal>")]` with the same
`(&self, Parameters<T>) -> Result<CallToolResult, ErrorData>` signature, and
`#[tool_router]` registers nothing that a `macro_rules!` would generate (memory
`rmcp-3-4-macro-constraints`). Each body is one `read_tool(self, move |c, s| { … })`
call whose closure already differs per tool (core module, `track()` handling,
envelope builder); the `*_data` helpers and the `run_read`/`run_write` pair were
folded away first, which is what the count below excludes. These 31 pairs are the
signature-and-attribute skeleton, not shared logic; the baseline moved from 235
to 266 as the MCP transport added three architecture tools.

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/mcp/tools_read.rs:40-50` method `architecture_scaffold` | `src/mcp/tools_read.rs:84-94` method `architecture_check` | 96.62% | rmcp-forced tool skeleton; the bodies call distinct architecture cores and renderers |
| `src/mcp/tools_read.rs:125-143` method `search` | `src/mcp/tools_read.rs:150-162` method `search_code` | 95.91% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:101-118` method `find` | `src/mcp/tools_read.rs:125-143` method `search` | 93.41% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:125-143` method `search` | `src/mcp/tools_read.rs:169-185` method `context` | 93.41% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_project_writes.rs:30-38` method `project_evidence` | `src/mcp/tools_project_writes.rs:44-52` method `project_propose` | 93.30% | rmcp-forced tool skeleton; the bodies call different project cores through `write_tool` |
| `src/mcp/tools_read.rs:101-118` method `find` | `src/mcp/tools_read.rs:169-185` method `context` | 93.26% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:40-50` method `architecture_scaffold` | `src/mcp/tools_read.rs:57-77` method `architecture_show` | 91.85% | rmcp-forced tool skeleton; the bodies call distinct architecture cores and renderers |
| `src/mcp/tools_read.rs:57-77` method `architecture_show` | `src/mcp/tools_read.rs:84-94` method `architecture_check` | 91.85% | rmcp-forced tool skeleton; the bodies call distinct architecture cores and renderers |
| `src/mcp/tools_read.rs:101-118` method `find` | `src/mcp/tools_read.rs:150-162` method `search_code` | 91.60% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:150-162` method `search_code` | `src/mcp/tools_read.rs:169-185` method `context` | 91.56% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:125-143` method `search` | `src/mcp/tools_read.rs:204-213` method `list` | 91.24% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:101-118` method `find` | `src/mcp/tools_read.rs:204-213` method `list` | 91.09% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:169-185` method `context` | `src/mcp/tools_read.rs:204-213` method `list` | 91.06% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:192-197` method `show` | `src/mcp/tools_read.rs:239-249` method `repos` | 90.63% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:204-213` method `list` | `src/mcp/tools_read.rs:256-265` method `recall_status` | 90.56% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:192-197` method `show` | `src/mcp/tools_read.rs:204-213` method `list` | 89.95% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:192-197` method `show` | `src/mcp/tools_read.rs:220-232` method `edges` | 89.92% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:220-232` method `edges` | `src/mcp/tools_read.rs:239-249` method `repos` | 89.92% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:125-143` method `search` | `src/mcp/tools_read.rs:256-265` method `recall_status` | 89.88% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:101-118` method `find` | `src/mcp/tools_read.rs:256-265` method `recall_status` | 89.73% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:125-143` method `search` | `src/mcp/tools_read.rs:220-232` method `edges` | 89.72% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:169-185` method `context` | `src/mcp/tools_read.rs:256-265` method `recall_status` | 89.69% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:101-118` method `find` | `src/mcp/tools_read.rs:220-232` method `edges` | 89.54% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:150-162` method `search_code` | `src/mcp/tools_read.rs:204-213` method `list` | 89.54% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:169-185` method `context` | `src/mcp/tools_read.rs:220-232` method `edges` | 89.50% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:204-213` method `list` | `src/mcp/tools_read.rs:220-232` method `edges` | 88.66% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:150-162` method `search_code` | `src/mcp/tools_read.rs:256-265` method `recall_status` | 88.43% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:150-162` method `search_code` | `src/mcp/tools_read.rs:220-232` method `edges` | 88.07% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:204-213` method `list` | `src/mcp/tools_read.rs:239-249` method `repos` | 87.76% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:220-232` method `edges` | `src/mcp/tools_read.rs:256-265` method `recall_status` | 87.42% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:192-197` method `show` | `src/mcp/tools_read.rs:256-265` method `recall_status` | 86.53% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |
| `src/mcp/tools_read.rs:239-249` method `repos` | `src/mcp/tools_read.rs:256-265` method `recall_status` | 86.53% | rmcp-forced tool skeleton; the bodies call different cores and envelopes |

### `src/config/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/config/paths.rs:89-91` method `sources_file` | `src/config/paths.rs:95-97` method `sources_lock_file` | 93.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/file.rs:170-183` method `apply` | `src/config/file.rs:222-235` method `apply` | 92.58% | the canonical parallel `Config`-section `apply()` case — each merges a different fixed field set, identical in shape because the sections are deliberately symmetric |
| `src/config/env.rs:72-77` function `env_pair` | `src/config/env.rs:82-87` function `env_triple` | 92.22% | different fixed sets of environment fields use the shared override helper |
| `src/config/env.rs:250-268` method `apply_rank_env` | `src/config/env.rs:272-299` method `apply_prune_env` | 91.53% | different fixed sets of environment fields use the shared override helper |
| `src/config/file.rs:170-183` method `apply` | `src/config/file.rs:273-292` method `apply` | 91.32% | the canonical parallel `Config`-section `apply()` case — each merges a different fixed field set, identical in shape because the sections are deliberately symmetric |
| `src/config/file.rs:222-235` method `apply` | `src/config/file.rs:273-292` method `apply` | 91.32% | the canonical parallel `Config`-section `apply()` case — each merges a different fixed field set, identical in shape because the sections are deliberately symmetric |
| `src/config/validate_knobs.rs:38-43` function `check_decay` | `src/config/validate_knobs.rs:49-54` function `check_unit_interval` | 90.40% | section-specific field validation; positive-count bounds and error formatting are shared. Moved out of `validate.rs` when that file hit the 300-line ceiling |
| `src/config/paths.rs:95-97` method `sources_lock_file` | `src/config/paths.rs:103-105` method `migration_lock_file` | 90.03% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:36-38` method `memories_dir` | `src/config/paths.rs:46-48` method `index_dir` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:69-71` method `config_file` | `src/config/paths.rs:75-77` method `auth_file` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:69-71` method `config_file` | `src/config/paths.rs:83-85` method `allowlist_file` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:69-71` method `config_file` | `src/config/paths.rs:89-91` method `sources_file` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:75-77` method `auth_file` | `src/config/paths.rs:83-85` method `allowlist_file` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:75-77` method `auth_file` | `src/config/paths.rs:89-91` method `sources_file` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:83-85` method `allowlist_file` | `src/config/paths.rs:89-91` method `sources_file` | 88.78% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/validate_knobs.rs:11-16` function `check_rrf_k` | `src/config/validate_knobs.rs:38-43` function `check_decay` | 87.64% | section-specific field validation; positive-count bounds and error formatting are shared. Moved out of `validate.rs` when that file hit the 300-line ceiling |
| `src/config/env.rs:17-28` function `env_parse` | `src/config/env.rs:72-77` function `env_pair` | 87.53% | different fixed sets of environment fields use the shared override helper |
| `src/config/env.rs:17-28` function `env_parse` | `src/config/env.rs:82-87` function `env_triple` | 86.91% | different fixed sets of environment fields use the shared override helper |
| `src/config/env.rs:226-231` method `apply_git_env` | `src/config/env.rs:302-309` method `apply_reinforce_env` | 86.26% | different fixed sets of environment fields use the shared override helper |
| `src/config/validate_knobs.rs:20-25` function `check_graph_hops` | `src/config/validate_knobs.rs:38-43` function `check_decay` | 86.09% | one bound per knob; the shapes rhyme because each is `if out of range { Err(msg) } else { Ok(()) }`, and each message names its own range |
| `src/config/validate_knobs.rs:30-35` function `check_positive_count` | `src/config/validate_knobs.rs:38-43` function `check_decay` | 86.09% | same: one bound per knob, each with its own message |
| `src/config/validate_knobs.rs:20-25` function `check_graph_hops` | `src/config/validate_knobs.rs:30-35` function `check_positive_count` | 85.79% | same: one bound per knob, each with its own message |
| `src/config/paths.rs:69-71` method `config_file` | `src/config/paths.rs:95-97` method `sources_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:69-71` method `config_file` | `src/config/paths.rs:103-105` method `migration_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:75-77` method `auth_file` | `src/config/paths.rs:95-97` method `sources_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:75-77` method `auth_file` | `src/config/paths.rs:103-105` method `migration_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:83-85` method `allowlist_file` | `src/config/paths.rs:95-97` method `sources_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:83-85` method `allowlist_file` | `src/config/paths.rs:103-105` method `migration_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/paths.rs:89-91` method `sources_file` | `src/config/paths.rs:103-105` method `migration_lock_file` | 85.66% | parallel path-join helpers, one per stored artifact under the same root — twin one-liners by construction |
| `src/config/validate.rs:143-166` method `check_rank_knobs` | `src/config/validate.rs:169-204` method `check_prune_knobs` | 85.23% | section-specific field validation; positive-count bounds and error formatting are shared |

### `src/serve/routes/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/serve/routes/graph_nodes.rs:129-138` function `node_detail` | `src/serve/routes/graph_nodes.rs:208-217` function `node_source` | 96.28% | short typed adapters supply distinct graph queries through shared query_response |
| `src/serve/routes/graph_nodes.rs:100-110` function `list` | `src/serve/routes/graph_nodes.rs:114-124` function `snapshot` | 95.60% | short typed adapters supply distinct graph queries through shared query_response |
| `src/serve/routes/jobs.rs:93-103` function `cancel` | `src/serve/routes/jobs.rs:136-143` function `get_one` | 93.44% | distinct event-stream transitions retain their lifecycle rules; payload serialization is shared |
| `src/serve/routes/jobs.rs:93-103` function `cancel` | `src/serve/routes/jobs.rs:154-168` function `events` | 92.17% | distinct event-stream transitions retain their lifecycle rules; payload serialization is shared |
| `src/serve/routes/jobs.rs:136-143` function `get_one` | `src/serve/routes/jobs.rs:154-168` function `events` | 90.97% | distinct event-stream transitions retain their lifecycle rules; payload serialization is shared |
| `src/serve/routes/jobs.rs:286-297` function `next_log` | `src/serve/routes/jobs.rs:302-311` function `try_next_log` | 90.89% | distinct event-stream transitions retain their lifecycle rules; payload serialization is shared |
| `src/serve/routes/sync_replica.rs:109-126` function `feed_route` | `src/serve/routes/sync_replica.rs:129-149` function `json_route` | 90.45% | SSE feed vs one-shot JSON page over the same replica core |
| `src/serve/routes/learning_console.rs:122-124` function `summary` | `src/serve/routes/learning_console.rs:169-176` function `proposals` | 89.36% | remaining handlers carry distinct containment, job, or mutation policy; simple reads share query_response |
| `src/serve/routes/repos_admin.rs:149-165` function `patch_repo` | `src/serve/routes/repos_admin.rs:170-182` function `archive_repo` | 89.26% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/repos_admin.rs:149-165` function `patch_repo` | `src/serve/routes/repos_admin.rs:205-217` function `disconnect_repo` | 88.52% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/repos_admin.rs:170-182` function `archive_repo` | `src/serve/routes/repos_admin.rs:205-217` function `disconnect_repo` | 88.09% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/graph_nodes.rs:129-138` function `node_detail` | `src/serve/routes/graph_nodes.rs:143-153` function `node_neighbors` | 87.65% | short typed adapters supply distinct graph queries through shared query_response |
| `src/serve/routes/activity_stream.rs:151-172` function `stream` | `src/serve/routes/activity_stream.rs:200-221` function `emit` | 87.34% | short route adapters: the unfold step and the event it yields, one owning the loop and the other the encoding |
| `src/serve/routes/graph_nodes.rs:143-153` function `node_neighbors` | `src/serve/routes/graph_nodes.rs:208-217` function `node_source` | 87.23% | short typed adapters supply distinct graph queries through shared query_response |
| `src/serve/routes/learning_console.rs:136-141` function `evals` | `src/serve/routes/learning_console.rs:230-235` function `expansions` | 86.86% | remaining handlers carry distinct containment, job, or mutation policy; simple reads share query_response |
| `src/serve/routes/memory_stores.rs:92-101` function `get_one` | `src/serve/routes/memory_stores.rs:115-132` function `patch` | 86.52% | read-one vs mutating patch; shared with_ctx/envelope shape, different HTTP contract |
| `src/serve/routes/graph_nodes.rs:100-110` function `list` | `src/serve/routes/graph_nodes.rs:143-153` function `node_neighbors` | 86.40% | short typed adapters supply distinct graph queries through shared query_response |
| `src/serve/routes/graph_nodes.rs:114-124` function `snapshot` | `src/serve/routes/graph_nodes.rs:143-153` function `node_neighbors` | 86.40% | short typed adapters supply distinct graph queries through shared query_response |
| `src/serve/routes/memory_stores.rs:79-88` function `list` | `src/serve/routes/memory_stores.rs:92-101` function `get_one` | 86.31% | parallel axum handlers over twin resources — same extract-validate-respond skeleton per route |
| `src/serve/routes/jobs.rs:93-103` function `cancel` | `src/serve/routes/jobs.rs:125-132` function `list` | 85.37% | distinct event-stream transitions retain their lifecycle rules; payload serialization is shared |

### `src/domains/sync/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/sync/code.rs:80-86` function `run_code_push` | `src/domains/sync/code.rs:92-98` function `run_code_push_if_moved` | 94.93% | parallel push/pull halves of the same exchange, symmetric by design |
| `src/domains/sync/code_repo_push.rs:126-132` function `cursor_matches` | `src/domains/sync/code_repo_push.rs:134-148` function `record_cursor` | 90.83% | read-compare of a stored cursor vs write of a new one; same row shape, opposite direction |
| `src/domains/sync/client_code.rs:18-35` function `fetch_code_manifest` | `src/domains/sync/client_code.rs:41-58` function `push_code_import` | 88.26% | parallel push/pull halves of the same exchange, symmetric by design |
| `src/domains/sync/client.rs:110-120` function `device_code` | `src/domains/sync/client.rs:123-137` function `poll_token` | 88.18% | parallel push/pull halves of the same exchange, symmetric by design |
| `src/domains/sync/auth_file.rs:120-142` method `load` | `src/domains/sync/auth_file.rs:156-168` method `load_usable` | 88.03% | parallel push/pull halves of the same exchange, symmetric by design |
| `src/domains/sync/client.rs:140-157` function `pull_changes` | `src/domains/sync/client.rs:263-273` function `fetch_manifest` | 87.51% | parallel push/pull halves of the same exchange, symmetric by design |
| `src/domains/sync/watch.rs:94-111` function `follow` | `src/domains/sync/watch.rs:119-148` function `channel_loop` | 86.79% | subscribe+drain loop vs the outer follow that opens the channel; cannot collapse without a mode flag |
| `src/domains/sync/auth_file.rs:201-204` method `clear` | `src/domains/sync/auth_file.rs:211-213` function `clear_stale_allowlist` | 86.75% | parallel push/pull halves of the same exchange, symmetric by design |

### `src/domains/retrieval/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/retrieval/score.rs:156-166` function `min_max_normalize` | `src/domains/retrieval/score.rs:183-189` function `max_normalize` | 91.94% | parallel scorers/normalizers over twin candidate shapes |
| `src/domains/retrieval/pipeline.rs:178-206` function `record_telemetry` | `src/domains/retrieval/pipeline.rs:256-267` function `record_query` | 91.84% | parallel scorers/normalizers over twin candidate shapes |
| `src/domains/retrieval/fuse.rs:54-56` function `rrf_multi` | `src/domains/retrieval/fuse.rs:65-67` function `rrf_multi_weighted` | 90.21% | parallel scorers/normalizers over twin candidate shapes |
| `src/domains/retrieval/scope.rs:70-75` method `window` | `src/domains/retrieval/scope.rs:126-128` method `window` | 89.43% | parallel scorers/normalizers over twin candidate shapes |
| `src/domains/retrieval/suggest.rs:96-107` function `expansions` | `src/domains/retrieval/suggest.rs:116-127` function `recent` | 89.21% | parallel scorers/normalizers over twin candidate shapes |
| `src/domains/retrieval/router.rs:254-271` function `route_vector_only` | `src/domains/retrieval/router.rs:340-358` function `route_lexical` | 88.14% | parallel scorers/normalizers over twin candidate shapes |
| `src/domains/retrieval/doc_route.rs:236-252` function `local_hit` | `src/domains/retrieval/doc_route.rs:261-284` function `shared_hit` | 87.96% | local vs remote document tables; keeping the two sides apart is the replication premise |
| `src/domains/retrieval/fuse.rs:32-34` function `rrf` | `src/domains/retrieval/fuse.rs:40-42` function `rrf_k` | 86.10% | parallel scorers/normalizers over twin candidate shapes |

### `src/serve/jobs/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/serve/jobs/worker.rs:31-45` function `spawn_job` | `src/serve/jobs/worker.rs:73-94` function `spawn_job_for` | 92.05% | parallel job-registry / worker arms, one per job kind |
| `src/serve/jobs/registry.rs:320-327` function `active_in` | `src/serve/jobs/registry.rs:332-340` function `refuse_active` | 90.33% | parallel job-registry / worker arms, one per job kind |
| `src/serve/jobs/worker.rs:31-45` function `spawn_job` | `src/serve/jobs/worker.rs:53-64` function `spawn_job_with_id` | 88.65% | parallel job-registry / worker arms, one per job kind |
| `src/serve/jobs/registry.rs:287-289` method `subscribe` | `src/serve/jobs/registry.rs:293-298` method `subscribe_progress` | 88.20% | parallel job-registry / worker arms, one per job kind |
| `src/serve/jobs/registry.rs:210-212` method `subscribe_log` | `src/serve/jobs/registry.rs:287-289` method `subscribe` | 86.61% | parallel job-registry / worker arms, one per job kind |
| `src/serve/jobs/events.rs:30-37` method `new` | `src/serve/jobs/events.rs:57-64` method `new` | 85.35% | parallel job-registry / worker arms, one per job kind |
| `src/serve/jobs/events.rs:30-37` method `new` | `src/serve/jobs/events.rs:80-82` method `new` | 85.35% | parallel job-registry / worker arms, one per job kind |

### `src/domains/integrations/setup/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/integrations/setup/apply.rs:105-115` function `git_hooks` | `src/domains/integrations/setup/apply.rs:118-135` function `index_code` | 89.66% | parallel per-integration setup arms, one per supported agent target |
| `src/domains/integrations/setup/plan.rs:63-75` function `data_dir` | `src/domains/integrations/setup/plan.rs:209-221` function `reinforce` | 87.43% | parallel per-integration setup arms, one per supported agent target |
| `src/domains/integrations/setup/apply.rs:54-58` function `data_dir` | `src/domains/integrations/setup/apply.rs:138-148` function `reinforce` | 86.78% | parallel per-integration setup arms, one per supported agent target |
| `src/domains/integrations/setup/apply.rs:34-50` function `one` | `src/domains/integrations/setup/apply.rs:118-135` function `index_code` | 86.55% | parallel per-integration setup arms, one per supported agent target |
| `src/domains/integrations/setup/apply.rs:118-135` function `index_code` | `src/domains/integrations/setup/apply.rs:138-148` function `reinforce` | 85.27% | parallel per-integration setup arms, one per supported agent target |
| `src/domains/integrations/setup/apply.rs:64-100` function `agent_host` | `src/domains/integrations/setup/apply.rs:118-135` function `index_code` | 85.06% | parallel per-integration setup arms, one per supported agent target |

### `src/serve/routes/memories/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/serve/routes/memories/search.rs:81-96` function `run_search` | `src/serve/routes/memories/search.rs:98-111` function `run_context` | 93.04% | parallel axum handlers over the twin memory read/write paths — same extract-validate-respond skeleton per route |
| `src/serve/routes/memories/search.rs:45-54` function `memories_search` | `src/serve/routes/memories/search.rs:57-69` function `context` | 92.87% | shared QueryOrJson extractor; still distinct command names and domain cores (search vs context) |
| `src/serve/routes/memories/write.rs:52-64` function `save` | `src/serve/routes/memories/write.rs:95-105` function `feedback` | 88.78% | parallel axum handlers over the twin memory read/write paths — same extract-validate-respond skeleton per route |
| `src/serve/routes/memories/edit.rs:59-70` function `update` | `src/serve/routes/memories/edit.rs:75-85` function `restore` | 88.03% | parallel axum handlers over the twin memory read/write paths — same extract-validate-respond skeleton per route |
| `src/serve/routes/memories/edit.rs:75-85` function `restore` | `src/serve/routes/memories/edit.rs:91-98` function `refresh_refs` | 86.44% | parallel axum handlers over the twin memory read/write paths — same extract-validate-respond skeleton per route |

### `src/domains/graph/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/graph/imports.rs:309-324` function `python_imports` | `src/domains/graph/imports.rs:328-351` function `go_imports` | 92.74% | Python and Go line parsers handle different syntax; import collection is shared |
| `src/domains/graph/graph_nodes.rs:300-314` function `fetch_top_symbols` | `src/domains/graph/graph_nodes.rs:319-328` function `fetch_cited_by` | 88.98% | parallel graph-page builders / row-to-DTO mappers |
| `src/domains/graph/doc_link.rs:101-139` function `resolve_markdown_links` | `src/domains/graph/doc_link.rs:157-176` function `resolve_document_id` | 88.71% | parallel graph-page builders / row-to-DTO mappers |
| `src/domains/graph/query.rs:39-48` function `build_code_graph` | `src/domains/graph/query.rs:54-66` function `build_graph_page` | 85.67% | parallel graph-page builders / row-to-DTO mappers |
| `src/domains/graph/doc_link.rs:22-46` function `derive_after_document` | `src/domains/graph/doc_link.rs:101-139` function `resolve_markdown_links` | 85.28% | parallel graph-page builders / row-to-DTO mappers |

### `src/utilities/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/utilities/error_code.rs:46-98` function `classify` | `src/utilities/error_code.rs:103-138` function `classify_project` | 93.46% | general Error→code map vs the project-scoped variant set; shared Class pairing |
| `src/utilities/file_lock.rs:60-67` method `try_acquire` | `src/utilities/file_lock.rs:73-89` method `acquire_within` | 88.07% | bounded wait vs non-blocking try; both open through the same helper |
| `src/utilities/id_list.rs:30-40` function `parse_id_csv` | `src/utilities/id_list.rs:48-65` function `parse_symbol_id_csv` | 87.88% | parallel CSV-parse-into-Vec helpers for two id kinds |
| `src/utilities/fetch.rs:47-64` function `download` | `src/utilities/fetch.rs:67-93` function `final_url` | 86.19% | parallel fetch helpers over twin response shapes |
| `src/utilities/fetch.rs:95-124` function `exchange_curl` | `src/utilities/fetch.rs:126-171` function `exchange_wget` | 85.63% | parallel fetch helpers over twin response shapes |

### `src/serve/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/serve/routes.rs:272-282` function `guard_mutating` | `src/serve/routes.rs:289-294` function `guard_job` | 93.07% | parallel request-plumbing helpers over twin shapes |
| `src/serve/router.rs:59-68` function `forbidden_response` | `src/serve/router.rs:72-77` function `unauthorized_response` | 89.56% | parallel request-plumbing helpers over twin shapes |
| `src/serve/routes.rs:308-317` function `respond` | `src/serve/routes.rs:322-327` function `accepted` | 86.59% | parallel request-plumbing helpers over twin shapes |
| `src/serve/envelope.rs:68-75` method `unauthorized` | `src/serve/envelope.rs:108-116` method `confirmation_required` | 85.57% | parallel refusal constructors over the shared `refusal` helper; `confirmation_required` names its message first |

### `src/cli/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/cli/sync_auto.rs:29-58` function `run` | `src/cli/sync_auto.rs:63-90` function `run_in_process` | 87.51% | retry-wake path vs in-process pass under COMEMORY_SYNC_DAEMON=0; shared JSON/TTY report tail, different work inside |
| `src/cli/capture.rs:85-112` function `run_sources` | `src/cli/capture.rs:135-149` function `run_session` | 86.69% | parallel CLI command helpers over twin inputs |
| `src/cli/watch.rs:90-108` function `follow_coordinator` | `src/cli/watch.rs:113-133` function `attach_once` | 86.41% | one attach attempt vs the retry loop around it; merge tried and made the count worse |

### `src/domains/maintenance/doctor/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/maintenance/doctor/checks.rs:62-64` function `warn` | `src/domains/maintenance/doctor/checks.rs:67-69` function `fail` | 93.35% | parallel maintenance checks, one per checked surface |
| `src/domains/maintenance/doctor/checks.rs:57-59` function `ok` | `src/domains/maintenance/doctor/checks.rs:62-64` function `warn` | 88.01% | status constructors that differ only in the Status variant they stamp |
| `src/domains/maintenance/doctor/checks.rs:57-59` function `ok` | `src/domains/maintenance/doctor/checks.rs:67-69` function `fail` | 88.01% | status constructors that differ only in the Status variant they stamp |

### `src/domains/memories/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/memories/save.rs:363-372` function `near_duplicate` | `src/domains/memories/save.rs:377-396` function `near_duplicate_inner` | 88.69% | parallel memory command handlers over twin operations |
| `src/domains/memories/list.rs:77-83` method `from` | `src/domains/memories/list.rs:115-128` method `from` | 85.16% | parallel memory command handlers over twin operations |

### `src/domains/code/ast/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/code/ast/extractor.rs:146-156` function `python_patterns` | `src/domains/code/ast/extractor.rs:158-169` function `go_patterns` | 91.82% | distinct Python and Go symbol pattern tables; language cache dispatch is shared |
| `src/domains/code/ast/pattern_cache.rs:31-40` function `cached` | `src/domains/code/ast/pattern_cache.rs:44-55` function `compile_patterns` | 85.31% | cache-hit vs. compile-miss halves of the same lookup |

### `src/domains/sync/memory_store/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/sync/memory_store/git.rs:157-167` function `unmerged_paths` | `src/domains/sync/memory_store/git.rs:191-194` function `head` | 86.43% | parallel store-backend operations over twin transports |
| `src/domains/sync/memory_store/git.rs:191-194` function `head` | `src/domains/sync/memory_store/git.rs:203-211` function `push` | 85.99% | parallel store-backend operations over twin transports |

### `src/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/serve.rs:199-201` method `repo` | `src/serve.rs:224-226` method `embed_cmd` | 87.31% | trivial parallel getter methods on `AppState` |

### `src/cli/output/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/cli/output/graph.rs:33-37` function `write_dot` | `src/cli/output/graph.rs:40-44` function `write_html` | 89.98% | parallel render helpers, one per output shape |

### `src/serve/routes/maint/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/serve/routes/maint/prune.rs:93-104` function `prune_apply` | `src/serve/routes/maint/prune.rs:143-151` function `gc` | 91.28% | parallel maintenance handlers — same extract-validate-respond skeleton per route |

### `src/domains/code/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/code/git_utils.rs:379-381` function `hook_installed` | `src/domains/code/git_utils.rs:397-400` function `hook_outdated` | 89.13% | parallel code-indexing helpers over twin inputs |

### `src/domains/documents/document/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/documents/document/extract.rs:70-84` function `extract_txt` | `src/domains/documents/document/extract.rs:86-95` function `extract_markdown` | 88.81% | parallel document extraction arms, one per source shape |

### `src/domains/learning/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/learning/learning_proposals.rs:181-189` function `parse_knobs` | `src/domains/learning/learning_proposals.rs:193-200` function `require_knobs` | 87.70% | parallel proposal builders over twin signal sources |

### `src/domains/retrieval/unified/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/retrieval/unified/fuse_domains.rs:213-226` function `code_hit` | `src/domains/retrieval/unified/fuse_domains.rs:229-248` function `doc_hit` | 89.78% | parallel per-domain fusion arms |

### `src/domains/integrations/install/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/integrations/install/skill_links.rs:131-134` function `symlink_dir` | `src/domains/integrations/install/skill_links.rs:138-143` function `symlink_dir` | 95.33% | two same-named helpers over different host skill roots; each owns its own dest layout |

### `src/domains/sync/daemon/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/sync/daemon/supervisor.rs:356-368` function `stop` | `src/domains/sync/daemon/supervisor.rs:371-386` function `remove` | 91.29% | stop keeps the unit file; remove stops then deletes — Intent::Restart needs the first |

### `src/domains/sync/drain/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/sync/drain/adopt_seeds.rs:31-47` function `advance` | `src/domains/sync/drain/adopt_seeds.rs:52-92` function `page` | 85.34% | the cursor-persisting loop and the one-page worker it calls; the same split `adopt.rs`'s `from_cursor`/`page` uses, just shorter, so the two short bodies read closer together than that pair's do |

### `src/domains/sync/replica/`

| Pair A | Pair B | Similarity | Remaining distinction |
| --- | --- | --- | --- |
| `src/domains/sync/replica/validate.rs:187-189` method `holds_for` | `src/domains/sync/replica/validate.rs:199-204` method `holds_for` | 95.59% | two same-named helpers over different hold kinds; each checks its own table |

## Continuing the cleanup

The remaining matches include distinct per-language parsers, per-table query
projections, short route adapters, and configuration sections with different
fields. Their structural similarity does not by itself establish that their
behavior can be merged. The rationale column records the distinction to inspect
before extracting another helper; it is not an exemption from the gate.

Shared query execution, AST traversal, environment overrides, validation errors,
HTTP query responses and job-event serialization have already been extracted.
Further cleanup should preserve each caller's ordering, errors and mutation
policy, then lower both this inventory and `dup-baseline.txt` in the same change.
