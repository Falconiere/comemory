# config/

**What belongs here:** layered configuration — defaults, the `config.toml`
file overlay, and `COMEMORY_*` environment overrides — plus `Paths`, the
single resolution of the on-disk data-directory layout, and the invariant
validation pass run after every layer is applied.

**What does NOT belong here:** reading an environment variable anywhere else
in the crate. Every other module reads a resolved `Config`/`Paths` value, not
`std::env` directly — that's what the `no-direct-env-var` guardrails rule
enforces (this folder is its documented exemption).

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `defaults.rs` | `default_memory_vector_dim` | Default-value functions backing `Config`'s `#[serde(default = "...")]` attributes |
| `env.rs` | `with_env` | `COMEMORY_*` env-var overrides — the outermost config layer |
| `file.rs` | `AutoReindexMode` | `Config` struct definitions, shipped defaults, and the `config.toml` overlay |
| `learning.rs` | `TuneConfig` | Learning-loop sections: `[tune]` grids, `[reinforce]`, `[bandit]` |
| `paths.rs` | `Paths` | Data-directory layout resolution (`resolve_data_dir` plus every derived path) |
| `retrieval.rs` | `RetrievalConfig` | The `[retrieval]` section and its file overlay |
| `validate.rs` | `validate` | Shared invariant pass over the fully layered config |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/config.rs` (`pub mod
<name>;`) and callers import concrete paths.
