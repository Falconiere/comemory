# `src/domains/`

**What belongs here:** business capabilities. Each folder owns one area of
behavior end to end — its models, its algorithms, and the command cores both
delivery adapters call — and appears as a sibling module file (`<name>.rs`) plus
its folder, declared from `src/domains.rs`.

**What does NOT belong here:** delivery. No file under `domains/` may import
`cli`, `serve`, `output`, or the legacy `api` command-core tree. A capability may
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
| `memories.rs` + [`memories/`](memories/README.md) | The memory lifecycle: the markdown record and its atomic store, and the save / delete / list / show / update / restore / trash / reference-refresh cores | #169 |
| `sync.rs` + [`sync/`](sync/README.md) | Organization authentication, platform push/pull and its wire protocol, the workspace channel, secret redaction, the opt-in daemon, and the Git memory store | #172 |

The remaining four capabilities named by the contract — `retrieval`,
`learning`, `maintenance`, `integrations` — arrive with #171, #173, #175 and
#176, and keep their legacy roots until then.
