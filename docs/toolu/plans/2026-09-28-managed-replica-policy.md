# Managed replica policy headers — Plan

**Date:** 2026-09-28
**Status:** Approved
**Spec:** docs/toolu/specs/2026-09-28-managed-replica-policy-design.md
**Topic:** Fix #356

## Evidence and approach
Issue #356, recalled memory 3032d382, session::split, and the real platform guard agree: policy negotiation uses repository-policy-v1 independently of replica-v1 data. Change only the replica managed transport argument. Retain revision and echo checks.

## Workstream summary
Reproduce against the real API, correct the transport, verify real engine compatibility, document the contract, and deliver after all gates.

## Steps (machine-readable)

```json
[
  {"id":"fix","title":"Correct managed replica policy and verify real transports","ac_refs":["AC-1","AC-2","AC-3"],"paths":["src/","tests/","Cargo.toml","Cargo.lock",".cargo/"],"input":"Real org-scoped auth supplied through COMEMORY_MANAGED_AUTH_FILE; real loopback engines","check":"cargo test --all-features --test replica_managed -- --ignored && cargo nextest run --all-features --lib -E 'test(domains::sync::drain::session::tests) | test(domains::sync::drain::transport::tests)'"},
  {"id":"gate","title":"Synchronize protocol docs and pass full quality gate","depends_on":["fix"],"ac_refs":["AC-1","AC-2","AC-3"],"paths":["."],"input":"Final repository sources, docs, and shipped migrations","check":"set -o pipefail; RUSTUP_TOOLCHAIN=1.95.0 bash scripts/check-all.sh | tee /tmp/comemory-issue356-gate.log"}
]
```

## Critical files
src/domains/sync/drain/session.rs; tests/replica_managed.rs; README.md; AGENTS.md; src/domains/sync/drain/README.md; docs/guides/managed-replica-testing.md.

## Verification
Record real API red→green output. Check exact policy header/revision echo, replica body version, wrong-header and stale-revision refusals, and real unmanaged engines. Run the umbrella gate. Before delivery: authenticated gh api user, non-default branch, installed required plugins; scoped commit, final plan-ledger run --verify, committed-diff toolu-review v2, verdict overall ready, push, default-branch PR, then pr-babysit handoff.

## Review
Reviewed against the delivery-flow checklist and supplied platform/source evidence. No open findings; Jev alignment judgment 0.82. All acceptance criteria have real-input checks; no public schema or signature changes.

## Deviations
The edit-time guard flagged the pre-existing open function at more than 50 lines after the header fix. Extracted its manifest decision/session construction into finish_open without changing the sequence or failure mapping; Jev chose this extraction over provisional defaults or ignoring the guard. The real-engine and managed regression checks cover the extraction.

The first green attempt reached a successful replica report but exposed an unsupported test assumption: the live platform advertises only replica-v1, with no kind@schema capabilities. Removed the nonempty-kind assumption and retained exact equality between reported kinds and advertised kind capabilities. Jev selected this contract-aligned assertion; verifier/platform behavior stays in scope unchanged.

Local review bounded the real CLI child to 120 seconds using the existing assert_cmd dependency (direct HTTP calls already have 30-second timeouts). Manual verification has no overall pass budget, so a live upstream must not hang the regression indefinitely. Jev supported this bounded test-only adjustment; Context7 confirmed the timeout/output API.
