# config/

**What belongs here:** layered configuration — defaults, the `config.toml`
file overlay, and `COMEMORY_*` environment overrides — plus `Paths`, the
single resolution of the on-disk data-directory layout, and the invariant
validation pass run after every layer is applied.

**What does NOT belong here:** reading an environment variable anywhere else
in the crate. Every other module reads a resolved `Config`/`Paths` value, not
`std::env` directly — that's what the `no-direct-env-var` guardrails rule
enforces (this folder is its documented exemption).

Nor, as a rule, a capability: `config` sits under every domain, so an import
back into one inverts the layering. Since #178 the architecture gate reports
any such edge as `shared layer dependency` unless it is declared in
`scripts/architecture-policy.json`'s `shared_domain_dependencies`, with a
written reason. This folder declares exactly one — `sync::skip_matcher`
returns `domains::sync::skip_repos::SkipMatcher`, because `validate` must
reject an invalid `[sync] skip_repos` glob at load and the matcher owns the
lowercase normalization that decides what a pattern means. Recompiling that
rule here would fork it. Default VALUES stay here, though: #178 moved
`SUPERSEDED_GRACE_DAYS` in from the prune rule that consumes it, so `defaults`
owns its own numbers.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `defaults.rs` | `default_memory_vector_dim` | Default-value functions backing `Config`'s `#[serde(default = "...")]` attributes |
| `env.rs` | `with_env` | `COMEMORY_*` env-var overrides — the outermost config layer |
| `file.rs` | `AutoReindexMode` | `Config` struct definitions, shipped defaults, and the `config.toml` overlay |
| `learning.rs` | `TuneConfig` | Learning-loop sections: `[tune]` grids, `[reinforce]`, `[bandit]` |
| `patch.rs` | `patch_config_file` | The one read-patch-atomically-write primitive over `config.toml`, shared by `tune --apply`, the `hooks` reinforce toggle, and the console-api config routes |
| `paths.rs` | `Paths` | Data-directory layout resolution (`resolve_data_dir` plus every derived path, including `auth_file`) |
| `retrieval.rs` | `RetrievalConfig` | The `[retrieval]` section and its file overlay |
| `sync.rs` | `SyncConfig` | The `[sync]` section — the inline push (`push_on_save`, `push_on_save_timeout`), the opt-in daemon's intervals, `skip_repos`, and `code_index` (the code-index push, on by default) |
| `validate.rs` | `validate` | Shared invariant pass over the fully layered config |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/config.rs` (`pub mod
<name>;`) and callers import concrete paths.
