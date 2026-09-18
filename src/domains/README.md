# `src/domains/`

**What belongs here:** business capabilities. Each folder owns one area of
behavior end to end — its models, its algorithms, and the command cores both
delivery adapters call — and appears as a sibling module file (`<name>.rs`) plus
its folder, declared from `src/domains.rs`.

**What does NOT belong here:** delivery. No file under `domains/` may import
`cli` (including the `cli::output` writers) or `serve`. A capability may
depend on `store`, `config`, `errors`, `prelude`, the named shared primitives in
[`utilities/`](../utilities/README.md), and — only through the directed table in
`scripts/architecture-policy.json` — another capability.

Filled one slice at a time by the
[#164 migration](../../docs/designs/2026-09-17-domain-first-migration.md). A
folder, README, or module declaration may never exist here without production
code owned by that capability: `scripts/architecture-check.sh` fails an empty
domain scaffold.

## Contents

One row per capability — its sibling module file and its folder together:

| Capability | Owns | Landed by |
| --- | --- | --- |
| `capture.rs` + [`capture/`](capture/README.md) | Client-side coding-session capture: transcript reading, client redaction attestation and receipts, explicit-save distillation, and the platform session and candidate-batch calls | #174 |
| `code.rs` + [`code/`](code/README.md) | AST extraction, code indexing, the repository inventory, Git hooks, reindex freshness | #167 |
| `documents.rs` + [`documents/`](documents/README.md) | Document extraction, the durable source registry, discovery, and document indexing | #168 |
| `graph.rs` + [`graph/`](graph/README.md) | The `edges` relation graph: reference and link derivation, co-change and import mining, PageRank and its projections, neighbor walks, co-activation, and the graph/edge command cores | #170 |
| `integrations.rs` + [`integrations/`](integrations/README.md) | Getting comemory working inside an agent host and getting a machine or a repo ready to use it: the embedded agent bundle and its atomic versioned marketplace, host validation with configuration-directory precedence and per-version, per-host installed markers, and the detect → plan → apply onboarding composition over its seven stable step ids — offline detection that never creates a missing database, a pure planner, and an apply that records each step's failure and carries on | #175 |
| `learning.rs` + [`learning/`](learning/README.md) | The learning loop: memory and code feedback counters with their provenance and identity rules, golden sets and metrics, reformulation mining, the deterministic, sampled and bandit searches over the ranking blend, and the feedback / eval / mine / tune / bandit cores plus the console read model and its knob proposals | #173 |
| `maintenance.rs` + [`maintenance/`](maintenance/README.md) | Corpus health, retention, repair and the binary upgrade: the structured doctor report and its probes, the trash/telemetry sweep and its retention policy, orphan / low-value / ghost-reference detection with its confirmed apply, the advisory near-duplicate cluster report, the atomic mirror rebuild and its preservation copy, re-embedding, the read-only stats and overview dashboards, and the CLI-only release-channel upgrade | #176 |
| `memories.rs` + [`memories/`](memories/README.md) | The memory lifecycle: the markdown record and its atomic store, and the save / delete / list / show / update / restore / trash / reference-refresh cores | #169 |
| `retrieval.rs` + [`retrieval/`](retrieval/README.md) | Hybrid search across memories, code and documents: the candidate legs and their fusion, the rerank priors and diversification, graph expansion, the time and domain scope, context bundles, pinned code-reference freshness, the score-explanation strip, and the search / search-code / context / find / suggest / retrieval-config cores | #171 |
| `sync.rs` + [`sync/`](sync/README.md) | Organization authentication, platform push/pull and its wire protocol, the workspace channel, secret redaction, the opt-in daemon, and the Git memory store | #172 |

Every capability named by the contract has now landed: #175 brought the last
one, `integrations`, out of the `api/` shell. What remains of the #164
migration is #177's storage-callback separation and #178's removal of the
emptied legacy layer — neither adds a capability here.
