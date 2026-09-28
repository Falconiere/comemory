# Managed replica policy headers — Design

**Date:** 2026-09-28
**Status:** Approved
**Author:** Codex
**Topic:** Fix #356

## Problem
Managed verification stops at session opening: replica-v1 is sent in the policy header, which the real platform refuses with sync_upgrade_required.

## Non-Goals
1. No platform guard changes, wire/schema changes, or negotiation redesign.
2. No credential, daemon, or unrelated sync changes.

## Architecture
Use SYNC_PROTOCOL for both managed transports in session::split. Keep REPLICA for manifest validation and replica payloads. Extract the existing manifest decision into finish_open to satisfy the edit-time 50-line guard, preserving the sequence and failure mapping. Preserve Transport's exact protocol/revision echo validation. Issue #356's live comparison and the platform repository-sync-policy.ts establish the separate contracts.

## Interfaces / Schema
No public signature or schema changes. X-Comemory-Sync-Protocol carries repository-policy-v1; X-Comemory-Policy-Revision carries the loaded revision. Manifest data.protocol remains replica-v1.

## Failure modes and edge cases
Unsupported policy headers and stale revisions still return 409. Missing/wrong response echoes still fail closed. Bare engines retain unmanaged negotiation. A live regression requires an explicitly supplied organization-scoped auth file and fails if unavailable; it runs against an isolated temporary store, without saving test memories remotely.

## Acceptance criteria
- **AC-1:** The current CLI against the real managed API produces a replica verification report, rather than exit 69.
- **AC-2:** With one real credential and current revision, the manifest accepts repository-policy-v1 and echoes it and the revision, while its body remains replica-v1; replica-v1 policy headers and stale revisions return their existing 409 codes.
- **AC-3:** Real unmanaged engine session and transport tests continue passing, including missing policy echo refusal.

## Acceptance evidence
AC-1: tests/replica_managed.rs uses COMEMORY_MANAGED_AUTH_FILE, copies auth into a private temporary directory, and runs the built CLI sync --action verify --json. Assert success and replica report shape. Run cargo test --all-features --test replica_managed -- --ignored.
AC-2: The same live test performs real authenticated manifest/status GETs and checks exact response headers, wire body and refusal codes.
AC-3: cargo nextest run --all-features --lib -E 'test(domains::sync::drain::tests) | test(domains::sync::drain::session::tests) | test(domains::sync::drain::transport::tests)'; use existing real-engine suites. Full bash scripts/check-all.sh gate is mandatory.

## Documentation impact
Clarify the existing policy/wire distinction in README.md, AGENTS.md, docs/guides, and src/domains/sync/drain/README.md. Document the opt-in real API regression and its credential requirement.

## Open Questions
None.

## Review
Reviewed against the delivery-flow checklist and supplied platform/source evidence. No open findings; Jev alignment judgment 0.82. All acceptance criteria have real-input checks; no public schema or signature changes.

## Observed evidence
On 2026-09-28 the real managed API regression reproduced CLI exit 69 with sync_upgrade_required before the fix, then reached CLI exit 0 after it. The live API accepted/echoed repository-policy-v1 and the current revision, kept replica-v1 in the manifest, and refused replica-v1 policy headers and the preceding revision with the expected 409 codes. Its capability list contains only replica-v1, so an empty kinds list is valid; the regression requires exact equality with advertised kind@schema capabilities. Ten real unmanaged engine session/transport tests passed. The full Rust 1.95 umbrella gate passed. Final committed-diff verification remains part of delivery.
