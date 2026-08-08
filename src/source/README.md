# source/

**What belongs here:** the durable document-source registry — TOML-backed
registration of external roots (`sources.toml`), the exclusive-flock guard
over concurrent read-modify-write cycles, the discovery walk and file
classification that decide which files are in scope, and the reconciler that
mirrors the registry into SQLite's `source_roots` table.

**What does NOT belong here:** document extraction itself. `source/` only
decides *which* files exist and are eligible; turning bytes into chunks is
`document/`'s job, and persisting the result is `store::documents` /
`store::document_fts`'s.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `classify.rs` | `Classification` | v1 extension allowlist + binary-content sniff, managed-directory exclusion |
| `discover.rs` | `Candidate` | Discovery walk over a registered source root: boundary/ignore/exclusion rules |
| `lock.rs` | `FileLock` | Exclusive advisory lock over a sibling lock file (generalized from `sources.toml.lock`; also consumed by `store::migrate`'s preflight snapshot) |
| `mirror.rs` | `MirrorReport` | Reconciles the TOML registry into the SQLite `source_roots` mirror |
| `registry.rs` | `Registry` | `sources.toml` load/save, overlap validation, atomic durability |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/source.rs` (`pub mod
<name>;`) and callers import concrete paths.
