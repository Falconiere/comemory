# Domain migration ownership inventory

This is the checked contract for #165 and subsequent #166–#178 slices.
The production list is `rg --files src -g '*.rs' -g '!src/**/tests/**' | sort`.
Only colocated test trees are excluded; the root test bridge remains inventoried.
Run `bash scripts/test-architecture-policy.sh` to validate the contract and its
negative cases, or add `--validate --policy <json> --inventory <markdown>` for
alternate copies. Resource columns contain resolved repository-relative paths
separated by `; `; `none` means no resource.

Owners are `domains::<capability>`, `delivery::cli`, `delivery::serve`,
`shared::config`, `shared::utilities`, `shared::root`, or `infrastructure::store`.
Targets are exact future files, not permission to create empty scaffolding.
API cores live directly under their capability. Concrete collision names are
code's `pattern_search`, graph's `view`, sync's API `exchange`, maintenance's
`consolidation` and `retention` algorithms, and learning's `evaluation` algorithms.
Children travel with their parent core. Stats feedback belongs to learning;
its shared vocabulary becomes `utilities::telemetry`.

The public column records each module reachable through public declarations
from `src/lib.rs`; its compatibility choice applies to its public items too.
`crate-root-alias` preserves a coherent old root export and public descendants
through `lib.rs`, never a directory barrel or an API/stats facade.
`preserve` keeps a retained path; `private` makes no library promise.

## Rust module path release note for 0.34.0

`0.34.0` shipped on 2026-09-18 with the thirteen capability slices #166-#177,
so every break below attributed to one of those is released. #178's own breaks
are NOT in it: `0.34.0` still carries the emptied `src/api.rs` shell and the
top-level `output` tree, both of which this slice removes, so they land in the
next minor release. They are listed under #178 below and marked `0.35.0`.

Version 0.34.0 emptied the technical `comemory::api` tree — every core moved to
`comemory::domains::<capability>` — and removed `comemory::stats`. Library users migrate to each row's target module.
Moved CLI utility modules and `serve::repo_root` also move to their named
utility paths; `cli::graph::nodes` moves to `domains::graph::nodes` with graph
assembly ownership. These are Rust module-path breaks only: CLI flags and output,
HTTP routes, JSON, wire protocols, ranking and persisted formats stay unchanged.
Breaking rows link to this release-note target before any source move.

#166 lands the first of these breaks. `comemory::{embed, fetch, http_error,
simhash}` keep resolving through crate-root aliases over their new
`comemory::utilities::` homes; the following paths move with no alias:

| Removed path | New path |
| --- | --- |
| `comemory::api::Ctx` | `comemory::utilities::context::Ctx` |
| `comemory::api::completions` | `comemory::cli::completion_script` |
| `comemory::cli::when` | `comemory::utilities::when` |
| `comemory::cli::ref_args` | `comemory::utilities::ref_args` |
| `comemory::output::page::Page` | `comemory::utilities::pagination::Page` |
| `comemory::output::search::PageMeta` | `comemory::utilities::pagination::PageMeta` |
| `comemory::retrieval::pipeline::PageWindow` | `comemory::utilities::pagination::PageWindow` |
| `comemory::output::search::SearchResult` | `comemory::retrieval::search_result::SearchResult` |
| `comemory::output::context::ContextResult` | `comemory::retrieval::context_result::ContextResult` |
| `comemory::output::search_code::SearchCodeResult` | `comemory::retrieval::code_search_result::SearchCodeResult` |
| `comemory::output::edges::EdgesResult` | `comemory::graph::edges_result::EdgesResult` |
| `comemory::output::consolidate::Report` | `comemory::consolidate::report::Report` |
| `comemory::output::prune::{Report, PruneRow}` | `comemory::prune::report::{Report, PruneRow}` |
| `comemory::source::lock::FileLock` | `comemory::utilities::file_lock::FileLock` |
| `comemory::serve::security::{resolve_within, contain_abs}` | `comemory::utilities::path_containment::{resolve_within, contain_abs}` |
| `comemory::api::index_code::ProgressSink` | `comemory::utilities::progress::ProgressSink` |
| `comemory::stats::feedback::{generate_query_id, is_valid_query_id}` | `comemory::utilities::query_id::{generate_query_id, is_valid_query_id}` |

#178 closes the series, and its breaks land in **0.35.0** rather than 0.34.0:
the emptied shell and the `output` tree were both still present in the 0.34.0
release. `comemory::output` keeps resolving through a crate-root alias over its
new `comemory::cli::output` home, so `comemory::output::{consolidate, context,
edges, graph, json, prune, search, search_code, tty}` are unaffected. The
following move with no alias:

| Removed path | New path |
| --- | --- |
| `comemory::api` (the emptied module itself) | removed; every core was already under `comemory::domains::<capability>` as of 0.34.0 |
| `comemory::output::edges::{Row, envelope}` | `comemory::domains::graph::edges_result::{Row, envelope}` |
| `comemory::utilities::when::scope_from_flags` | `comemory::domains::retrieval::scope::scope_from_flags` |

`comemory::serve::serve`'s third parameter changes from `json: bool` to
`ready: &dyn Fn(Ready<'_>) -> Result<()>`, and `comemory::serve::Ready` is new:
the startup banner moved to `cli::serve`, so the server no longer writes to
stdout and `--json`, a CLI global, no longer crosses into it. The banner's own
JSON — `{url, port, token, read_only}`, in that key order — is unchanged, and
`tests/cli__serve.rs` pins it.

#168 moves the document capability. `comemory::{document, source}` keep
resolving through crate-root aliases over their new
`comemory::domains::documents::` homes; the three command cores move with no
alias:

| Removed path | New path |
| --- | --- |
| `comemory::api::index` | `comemory::domains::documents::index` |
| `comemory::api::sources` | `comemory::domains::documents::sources` |
| `comemory::api::unindex` | `comemory::domains::documents::unindex` |

`comemory::stats::{source, target}` and `comemory::stats::feedback`'s
`PROV_*` / sentinel query-id consts were crate-private and stay crate-private
in `utilities::telemetry`, so they are not a library break.

#167 lands the second set. `comemory::ast` and `comemory::git_utils` keep
resolving through crate-root aliases over their new `comemory::domains::code::`
homes; the following move with no alias:

| Removed path | New path |
| --- | --- |
| `comemory::api::ast` | `comemory::domains::code::pattern_search` |
| `comemory::api::index_code` (+ `::walk`) | `comemory::domains::code::index_code` (+ `::walk`) |
| `comemory::api::ingest_code` | `comemory::domains::code::ingest_code` |
| `comemory::api::index_runs` | `comemory::domains::code::index_runs` |
| `comemory::api::repos` (+ `::git_state`) | `comemory::domains::code::repos` (+ `::git_state`) |
| `comemory::api::repo_admin` | `comemory::domains::code::repo_admin` |
| `comemory::api::hooks` | `comemory::domains::code::hooks` |
| `comemory::api::install_hooks` | `comemory::domains::code::install_hooks` |
| `comemory::serve::repo_root` | `comemory::utilities::repo_root` |
| `comemory::cli::lazy_reindex::{LastTrigger, should_reindex, parse_trigger, encode_trigger}` | `comemory::domains::code::reindex_policy::{…}` |
| `comemory::cli::graph::parse_id` | `comemory::utilities::repo_root::parse_id` |

`comemory::serve::RootOverrides` is **unchanged**: the re-export predates #167
and now points at `utilities::repo_root::RootOverrides`.
`comemory::cli::lazy_reindex` itself is retained — only the four items above
left it. `parse_id` moves for the same reason `repo_root` does: the shared
resolver decodes `file:<repo>:<path>` ids, and a shared primitive may not
import a delivery module. `comemory::cli::graph` itself is retained.

#169 lands the third set. `comemory::memory` keeps resolving through a
crate-root alias over its new `comemory::domains::memories` home, so
`comemory::memory::{frontmatter, id, prior, references, slug, store}` and the
items they re-export are unchanged; the eight command cores move with no alias:

| Removed path | New path |
| --- | --- |
| `comemory::api::save` | `comemory::domains::memories::save` |
| `comemory::api::delete` | `comemory::domains::memories::delete` |
| `comemory::api::list` | `comemory::domains::memories::list` |
| `comemory::api::show` | `comemory::domains::memories::show` |
| `comemory::api::update` | `comemory::domains::memories::update` |
| `comemory::api::restore` | `comemory::domains::memories::restore` |
| `comemory::api::trash` | `comemory::domains::memories::trash` |
| `comemory::api::refresh_refs` | `comemory::domains::memories::refresh_refs` |

`cli::delete::{soft_delete, mirror_soft_delete}` and
`output::search::{title_of, abs_path}` were crate-private and stay
crate-private in `domains::memories::delete` and `domains::memories::nav`, so
they are not a library break. `comemory::cli::delete` and
`comemory::output::search` themselves are retained.

#172 lands the fourth set. `comemory::sync` and `comemory::cloud` keep
resolving through crate-root aliases over their new `comemory::domains::sync`
home, so `comemory::sync::{auth_file, client, client_code, code, code_plan,
daemon, daemon_templates, daemon_unit, initial, pull, push, push_on_save,
redact, skip_repos, verify}`, `comemory::cloud::{api_url, device}` and the
items they re-export are unchanged; the two API cores move with no alias:

| Removed path | New path |
| --- | --- |
| `comemory::api::sync` (+ `::{changes, code_import, code_manifest, code_types, import, manifest, types}`) | `comemory::domains::sync::exchange` (+ the same children) |
| `comemory::api::memory_store` | `comemory::domains::sync::memory_store` |

`cli::watch::backoff_delay` was public only so its own test could reach it and
moves with the watch service to `domains::sync::watch::backoff_delay`;
`comemory::cli::watch` itself is retained, now holding the clap arguments, the
launch and the reporting. `domains::sync::{login, manual, watch}` are new
public modules rather than moved ones: they hold the non-presentation halves
of `cli::{auth, sync, watch}`, which were never a library surface.

`comemory::index` is **removed with no replacement**. It has been an empty
module since v0.2 (indexing moved to `store::vector` / `store::fts`), so nothing
resolvable was lost; a `crate-root-alias` needs a coherent item to alias, and an
empty module has none.

#170 lands the third set. `comemory::graph` keeps resolving through a crate-root
alias over its new `comemory::domains::graph::` home, so every algorithm module
under it — `coactivate`, `cochange`, `cross_link`, `derived`, `doc_link`,
`edges_result`, `imports`, `materialize`, `memory_rank`, `neighbors`,
`pagerank` — is unaffected. The following move with no alias:

| Removed path | New path |
| --- | --- |
| `comemory::api::graph` | `comemory::domains::graph::view` |
| `comemory::api::graph_nodes` | `comemory::domains::graph::graph_nodes` |
| `comemory::api::graph_recompute` | `comemory::domains::graph::graph_recompute` |
| `comemory::api::edges` | `comemory::domains::graph::edges` |
| `comemory::cli::graph::{Rel, build_code_graph, build_graph_page}` | `comemory::domains::graph::query::{Rel, build_code_graph, build_graph_page}` |
| `comemory::cli::graph::nodes` | `comemory::domains::graph::nodes` |
| `comemory::output::graph::{Node, Edge, CodeGraph, GraphPage}` | `comemory::domains::graph::code_graph::{Node, Edge, CodeGraph, GraphPage}` |

`comemory::output::graph` is retained and keeps its JSON, DOT and HTML writers
and the `graph_template.html` asset; only the four data types move, so the
renderer now imports the model rather than owning it. `comemory::cli::graph` is
likewise retained, holding the clap surface (`Args`, `Format`, `run`) alone.

#171 moves the retrieval capability. `comemory::retrieval` keeps resolving
through a crate-root alias over its new `comemory::domains::retrieval` home, so
every module under it — `bundle`, `code_prior`, `code_ref_collect`,
`code_ref_fetch`, `code_ref_status`, `code_rerank`, `code_route`,
`code_search`, `code_search_result`, `context_result`, `diversify`,
`doc_route`, `fuse`, `graph_route`, `pipeline`, `rerank`, `router`, `scope`,
`score`, `search_result` and `unified` (+ `unified::fuse_domains`) — is
unaffected. The following move with no alias:

| Removed path | New path |
| --- | --- |
| `comemory::api::search` | `comemory::domains::retrieval::search` |
| `comemory::api::search_code` | `comemory::domains::retrieval::search_code` |
| `comemory::api::context` | `comemory::domains::retrieval::context` |
| `comemory::api::find` | `comemory::domains::retrieval::find` |
| `comemory::api::suggest` | `comemory::domains::retrieval::suggest` |
| `comemory::api::config_retrieval` | `comemory::domains::retrieval::config_retrieval` |
| `comemory::output::explain` | `comemory::domains::retrieval::explain` |
| `comemory::output::search::{Row, Envelope, envelope}` | `comemory::domains::retrieval::search_result::{Row, Envelope, envelope}` |
| `comemory::output::search::ScopeEcho` | `comemory::domains::retrieval::scope::ScopeEcho` |
| `comemory::output::search_code::{Row, Envelope, envelope}` | `comemory::domains::retrieval::code_search_result::{Row, Envelope, envelope}` |
| `comemory::output::context::{Envelope, envelope}` | `comemory::domains::retrieval::context_result::{Envelope, envelope}` |

`comemory::output::{search, search_code, context}` are retained and keep their
`emit` / `write_tty` writers; only the shapes both transports serialize move,
because `serve::routes::{memories::search, code, search}` called those
`envelope` builders directly and `output` is CLI presentation only.
`output::search::source_label` was crate-private and becomes the public
`domains::retrieval::search_result::source_label`, since two capability modules
and both TTY writers now share it. `comemory::cli::search_only` is private and
retained: it keeps the clap `OnlyDomain` `ValueEnum`, its
`From<OnlyDomain> for Domain` conversion and the interim document-only path,
and delegates the resolution policy to the new transport-neutral
`domains::retrieval::scope::resolve_domains`, which takes `&[Domain]` rather
than the clap enum.

#173 moves the learning capability. `comemory::eval` keeps resolving through a
crate-root alias over its new `comemory::domains::learning::evaluation` home, so
`comemory::eval::{bandit, bandit_rng, golden, metrics, mine, runner, tune,
tune_sample}` and the items they re-export are unchanged. `comemory::stats` is
**removed with no alias** — a `crate-root-alias` needs a coherent old root
export, and `stats` was a technical grouping of three unrelated halves rather
than a capability. The following move:

| Removed path | New path |
| --- | --- |
| `comemory::api::feedback` | `comemory::domains::learning::feedback` |
| `comemory::api::eval` | `comemory::domains::learning::eval` |
| `comemory::api::mine` | `comemory::domains::learning::mine` |
| `comemory::api::tune` | `comemory::domains::learning::tune` |
| `comemory::api::bandit` | `comemory::domains::learning::bandit` |
| `comemory::api::learning` | `comemory::domains::learning::console` |
| `comemory::api::learning_proposals` | `comemory::domains::learning::learning_proposals` |
| `comemory::stats::feedback` | `comemory::domains::learning::feedback_tracking` |
| `comemory::stats::code_feedback` | `comemory::domains::learning::code_feedback` |
| `comemory::stats::sqlite::StatsDb` | `comemory::domains::learning::telemetry::StatsDb` |
| `comemory::stats::sqlite::StatsDb::record_index_failure` | `comemory::store::index_failures::record` |
| `comemory::stats::sqlite::StatsDb::index_failure_count` | `comemory::store::index_failures::count` |
| `comemory::stats::sqlite::StatsDb::last_index_failure` | `comemory::store::index_failures::latest` |

`api::learning` becomes `console` rather than keeping its name: a
`domains::learning::learning` module is rejected by `clippy::module_inception`
under `-D warnings`, and Binding Rule 5 forbids an `#[allow]`. It is the fifth
collision name the contract resolves, beside code's `pattern_search`, graph's
`view`, sync's `exchange` and learning's own `evaluation`.

The three `StatsDb` index-failure methods move to the store rather than into
the capability. They are indexing bookkeeping — an append-only log of swallowed
dense-embed/FTS upsert failures — with no production caller, no feedback table,
no query id and no provenance; they sat on `StatsDb` only because it was the
handle when the chokepoint work moved their SQL out. `store::index_failures`
now owns the whole concern including the ISO 8601 timestamp and the `usize`
clamp, so `StatsDb` is the three-method connection handle the feedback writers
borrow. `stats::feedback::record_implicit_used` was crate-private and stays
crate-private at `domains::learning::feedback_tracking::record_implicit_used`,
still taking a bare `&Connection` so `domains::graph::materialize` keeps
minting the co-activation reward inside the transaction that also advances its
cursor.

#176 moves the maintenance capability. `comemory::{prune, consolidate,
upgrade}` keep resolving through crate-root aliases over their new
`comemory::domains::maintenance::{retention, consolidation, upgrade}` homes, so
`comemory::prune::{low_value, orphans, stale_code}`,
`comemory::consolidate::{cluster, keeper}` and
`comemory::upgrade::{channel, installer, release, version}` are unchanged. The
two report models leave those aliases: the contract places each capability's
owned value as a **sibling** of its algorithm folder, so
`comemory::{prune, consolidate}::report` is a removal with no alias. The nine
command cores move with no alias:

| Removed path | New path |
| --- | --- |
| `comemory::api::doctor` (+ `::backup`, `::checks`, `::system`) | `comemory::domains::maintenance::doctor` (+ same children) |
| `comemory::api::gc` | `comemory::domains::maintenance::gc` |
| `comemory::api::gc_policy` | `comemory::domains::maintenance::gc_policy` |
| `comemory::api::prune` | `comemory::domains::maintenance::prune` |
| `comemory::api::consolidate` | `comemory::domains::maintenance::consolidate` |
| `comemory::api::rebuild` (+ `::copy`) | `comemory::domains::maintenance::rebuild` (+ `::copy`) |
| `comemory::api::reembed` | `comemory::domains::maintenance::reembed` |
| `comemory::api::stats` | `comemory::domains::maintenance::stats` |
| `comemory::api::overview` | `comemory::domains::maintenance::overview` |
| `comemory::prune::report::{Report, PruneRow}` | `comemory::domains::maintenance::retention_report::{Report, PruneRow}` |
| `comemory::consolidate::report::Report` | `comemory::domains::maintenance::consolidation_report::Report` |

`prune` becomes `retention` and `consolidate` becomes `consolidation` because
each command core keeps the subcommand's own name (`maintenance::prune`,
`maintenance::consolidate`) and an algorithm root may not share it. Both were
pre-reserved by this contract's preamble. `upgrade` needs no rename:
`maintenance::upgrade` beside `maintenance/upgrade/` is the ordinary sibling
shape, not `clippy::module_inception`.

Nothing about behavior moves with them. `upgrade` remains in
`serve::routes::meta::CLI_ONLY` beside `serve` and `auth`, so no HTTP endpoint
can replace the running binary; the `ATTACH`/copy/`DETACH` preservation unit,
the pre-rebuild `VACUUM INTO` snapshot and the migration chain stay in
`store`; and `doctor` keeps the read-only, must-not-create-the-database
semantics that `domains::integrations::setup::detect` depends on —
`setup_runtime_dependencies` simply retargets that declared edge from
`crate::api::doctor` to `crate::domains::maintenance::doctor`.

#175 moves the integrations capability, emptying `src/api/` of the last two
command cores. Neither has a crate-root alias to preserve, because neither was
ever a root module, so both are removals:

| Removed path | New path |
| --- | --- |
| `comemory::api::install` | `comemory::domains::integrations::install` |
| `comemory::api::setup` | `comemory::domains::integrations::setup` |
| `comemory::api::setup::apply` | `comemory::domains::integrations::setup::apply` |
| `comemory::api::setup::detect` | `comemory::domains::integrations::setup::detect` |
| `comemory::api::setup::plan` | `comemory::domains::integrations::setup::plan` |

Nothing about behavior moves with them. Both stay CLI-only in
`serve::routes::meta::CLI_ONLY` and in the `tests/api__parity.rs` exceptions;
the seven step ids, the offline detection, the report-only `cloud-auth` and
`index-docs` steps and the connection-free installer are unchanged, and the
authored assets stay in the repository-root `integrations/agent/` tree, which
`install/bundle.rs` embeds one directory deeper than before. The two
`setup_runtime_dependencies` rows that named `crate::api::install` are
retargeted onto `crate::domains::integrations::install`; they stay recorded
even though they are now intra-capability, because that list exists to record
detection's and application's exact runtime delegations.

With `src/api/` holding no row, the ledger's `src/api/` core-ownership check
had nothing left to constrain — `all` over an empty selection is true — so
#175 replaces it with a capability-ownership rule anchored on the capability
folder instead: a row under `src/domains/<capability>/` is owned by that
capability, and a row owned by a capability lives inside it. Every
`src/domains/` row is subject to it, and it fails as
`capability ownership mismatch`.

#178 closes the series. It deletes the emptied `src/api.rs` shell with its
`legacy_modules` entry and its ledger row, moves the `output` tree under
`cli/`, and widens that ownership rule into one total map from path to owner —
`src/domains/<cap>/**` to `domains::<cap>`, `src/cli/**` to `delivery::cli`,
`src/serve/**` to `delivery::serve`, `src/store/**` to
`infrastructure::store`, `src/config/**` and `src/utilities/**` to their
`shared::` owners, every other `src/*.rs` to `shared::root`. The capability
rule constrained 204 of `main`'s 411 rows and left 207 unconstrained, which is
how a file at the source root (`src/output.rs`) carried `delivery::cli` and a
file under `src/cli/` (`src/cli/pagination.rs`) carried `shared::utilities`.
Both are corrected here, the second by moving the owner rather than the file:
it holds nothing but the flattened clap `PaginationArgs`. The rule was measured
against the live ledger before it was written — 399 of 411 rows already
satisfied it, and the twelve that did not are exactly the rows this slice
deletes, moves or relabels: `src/api.rs`, `src/cli/pagination.rs`, and the ten
`src/output*` rows.

A second gap closed with it: `config` and `utilities` sources were never
subject to any dependency rule, so seven references back into a capability had
accumulated unnoticed, across five files. Two are removed — `scope_from_flags`
moves to `domains::retrieval::scope`, which owns `TimeScope`, and the prune
rule's `SUPERSEDED_GRACE_DAYS` moves into `config::defaults`, which owns its
own default values. The remaining five are declared in the new
`shared_domain_dependencies` list, each with a written reason, and anything
undeclared now fails as `shared layer dependency`.

After this slice every ledger row reads `issue = retain` with `path == target`:
the #164 migration has nothing outstanding.

## Baseline compatibility

Merged #162/#163 are baseline: common-directory hooks, unseen/custom worktree
labels, setup's seven stable step IDs, offline detection that never creates a
missing database, report-only cloud/document steps, and rendering every step's
failure before exit 69 must remain unchanged.

## Policy schema

`legacy_modules` lists `{module, owner, issue}` for staged roots that must
move; #178 empties it, and its fixtures seed a well-formed entry rather than
mutating an empty array, which would auto-vivify a half-formed one and assert
nothing. `shared_domain_dependencies` lists `{source, target, owner, reason}`
for the edges `config`/`utilities` keep into a capability; it must stay
non-empty for the same reason, and every entry carries a written
justification.
`owner_dependencies` lists directed `{source, target}` capability owner pairs.
`setup_runtime_dependencies` separately records exact `{source, target, owner}`
runtime edges for detection and application, not migration prerequisites.
`legacy_edges` and `store_callbacks` contain `{source, target, class, issue}`;
only #166, #167, #169, #170, and #177 remove these edges. Delivery targets name
canonical modules except root CLI helpers, which name exact items. Store
callbacks name exact functions/constructors. Passive models use `{source, target}`
and name exact imported types. No wildcard applies. ast-grep excludes comments
and strings when checking edge presence. `store::memory_purge` uses the shared
telemetry vocabulary; SimHash is a shared utility, so neither is a domain callback.

## File ledger

| path | public path | test bridge | assets | owner | target | issue |
| --- | --- | --- | --- | --- | --- | --- |
| src/cli.rs | comemory::cli; preserve | none | none | delivery::cli | src/cli.rs | retain |
| src/cli/ast.rs | comemory::cli::ast; preserve | none | none | delivery::cli | src/cli/ast.rs | retain |
| src/cli/auth.rs | comemory::cli::auth; preserve | none | none | delivery::cli | src/cli/auth.rs | retain |
| src/cli/auth_render.rs | comemory::cli::auth_render; preserve | none | none | delivery::cli | src/cli/auth_render.rs | retain |
| src/cli/bandit.rs | comemory::cli::bandit; preserve | none | none | delivery::cli | src/cli/bandit.rs | retain |
| src/cli/capture.rs | comemory::cli::capture; preserve | none | none | delivery::cli | src/cli/capture.rs | retain |
| src/cli/completion_script.rs | comemory::cli::completion_script; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/cli/tests/completion_script.rs | none | delivery::cli | src/cli/completion_script.rs | retain |
| src/cli/completions.rs | comemory::cli::completions; preserve | none | none | delivery::cli | src/cli/completions.rs | retain |
| src/cli/consolidate.rs | comemory::cli::consolidate; preserve | none | none | delivery::cli | src/cli/consolidate.rs | retain |
| src/cli/context.rs | comemory::cli::context; preserve | none | none | delivery::cli | src/cli/context.rs | retain |
| src/cli/delete.rs | comemory::cli::delete; preserve | none | none | delivery::cli | src/cli/delete.rs | retain |
| src/cli/distill.rs | comemory::cli::distill; preserve | none | none | delivery::cli | src/cli/distill.rs | retain |
| src/cli/doctor.rs | comemory::cli::doctor; preserve | none | none | delivery::cli | src/cli/doctor.rs | retain |
| src/cli/edges.rs | comemory::cli::edges; preserve | none | none | delivery::cli | src/cli/edges.rs | retain |
| src/cli/benchmark.rs | comemory::cli::benchmark; preserve | none | none | delivery::cli | src/cli/benchmark.rs | retain |
| src/cli/eval.rs | comemory::cli::eval; preserve | none | none | delivery::cli | src/cli/eval.rs | retain |
| src/cli/feedback.rs | comemory::cli::feedback; preserve | none | none | delivery::cli | src/cli/feedback.rs | retain |
| src/cli/find.rs | comemory::cli::find; preserve | none | none | delivery::cli | src/cli/find.rs | retain |
| src/cli/gc.rs | comemory::cli::gc; preserve | none | none | delivery::cli | src/cli/gc.rs | retain |
| src/cli/graph.rs | comemory::cli::graph; preserve | none | none | delivery::cli | src/cli/graph.rs | retain |
| src/cli/hooks.rs | comemory::cli::hooks; preserve | none | none | delivery::cli | src/cli/hooks.rs | retain |
| src/cli/index.rs | comemory::cli::index; preserve | none | none | delivery::cli | src/cli/index.rs | retain |
| src/cli/index_code.rs | comemory::cli::index_code; preserve | none | none | delivery::cli | src/cli/index_code.rs | retain |
| src/cli/ingest_code.rs | comemory::cli::ingest_code; preserve | none | none | delivery::cli | src/cli/ingest_code.rs | retain |
| src/cli/install.rs | comemory::cli::install; preserve | none | none | delivery::cli | src/cli/install.rs | retain |
| src/cli/install_hooks.rs | comemory::cli::install_hooks; preserve | none | none | delivery::cli | src/cli/install_hooks.rs | retain |
| src/cli/export_dataset.rs | comemory::cli::export_dataset; preserve | none | none | delivery::cli | src/cli/export_dataset.rs | retain |
| src/cli/judge.rs | comemory::cli::judge; preserve | none | none | delivery::cli | src/cli/judge.rs | retain |
| src/cli/lazy_reindex.rs | comemory::cli::lazy_reindex; preserve | none | none | delivery::cli | src/cli/lazy_reindex.rs | retain |
| src/cli/list.rs | comemory::cli::list; preserve | none | none | delivery::cli | src/cli/list.rs | retain |
| src/cli/mcp.rs | comemory::cli::mcp; preserve | none | none | delivery::cli | src/cli/mcp.rs | retain |
| src/cli/mine.rs | comemory::cli::mine; preserve | none | none | delivery::cli | src/cli/mine.rs | retain |
| src/cli/off_runtime.rs | comemory::cli::off_runtime; preserve | src/cli/tests/off_runtime.rs | none | delivery::cli | src/cli/off_runtime.rs | retain |
| src/cli/output.rs | comemory::cli::output; crate-root-alias | none | none | delivery::cli | src/cli/output.rs | retain |
| src/cli/output/consolidate.rs | comemory::cli::output::consolidate; crate-root-alias | none | none | delivery::cli | src/cli/output/consolidate.rs | retain |
| src/cli/output/context.rs | comemory::cli::output::context; crate-root-alias | src/cli/output/tests/context.rs | none | delivery::cli | src/cli/output/context.rs | retain |
| src/cli/output/edges.rs | comemory::cli::output::edges; crate-root-alias | src/cli/output/tests/edges.rs | none | delivery::cli | src/cli/output/edges.rs | retain |
| src/cli/output/graph.rs | comemory::cli::output::graph; crate-root-alias | src/cli/output/tests/graph.rs | src/cli/output/graph_template.html | delivery::cli | src/cli/output/graph.rs | retain |
| src/cli/output/json.rs | comemory::cli::output::json; crate-root-alias | none | none | delivery::cli | src/cli/output/json.rs | retain |
| src/cli/output/prune.rs | comemory::cli::output::prune; crate-root-alias | src/cli/output/tests/prune.rs | none | delivery::cli | src/cli/output/prune.rs | retain |
| src/cli/output/search.rs | comemory::cli::output::search; crate-root-alias | none | none | delivery::cli | src/cli/output/search.rs | retain |
| src/cli/output/search_code.rs | comemory::cli::output::search_code; crate-root-alias | none | none | delivery::cli | src/cli/output/search_code.rs | retain |
| src/cli/output/tty.rs | comemory::cli::output::tty; crate-root-alias | src/cli/output/tests/tty.rs | none | delivery::cli | src/cli/output/tty.rs | retain |
| src/cli/pagination.rs | comemory::cli::pagination; preserve | none | none | delivery::cli | src/cli/pagination.rs | retain |
| src/cli/prune.rs | comemory::cli::prune; preserve | none | none | delivery::cli | src/cli/prune.rs | retain |
| src/cli/rebuild.rs | comemory::cli::rebuild; preserve | none | none | delivery::cli | src/cli/rebuild.rs | retain |
| src/cli/recall_status.rs | comemory::cli::recall_status; preserve | none | none | delivery::cli | src/cli/recall_status.rs | retain |
| src/cli/repos.rs | comemory::cli::repos; preserve | none | none | delivery::cli | src/cli/repos.rs | retain |
| src/cli/save.rs | comemory::cli::save; preserve | none | none | delivery::cli | src/cli/save.rs | retain |
| src/cli/search.rs | comemory::cli::search; preserve | none | none | delivery::cli | src/cli/search.rs | retain |
| src/cli/search_code.rs | comemory::cli::search_code; preserve | none | none | delivery::cli | src/cli/search_code.rs | retain |
| src/cli/search_only.rs | private | src/cli/tests/search_only.rs | none | delivery::cli | src/cli/search_only.rs | retain |
| src/cli/serve.rs | comemory::cli::serve; preserve | none | none | delivery::cli | src/cli/serve.rs | retain |
| src/cli/setup.rs | comemory::cli::setup; preserve | none | none | delivery::cli | src/cli/setup.rs | retain |
| src/cli/setup/render.rs | comemory::cli::setup::render; preserve | src/cli/setup/tests/render.rs | none | delivery::cli | src/cli/setup/render.rs | retain |
| src/cli/setup/wizard.rs | comemory::cli::setup::wizard; preserve | none | none | delivery::cli | src/cli/setup/wizard.rs | retain |
| src/cli/show.rs | comemory::cli::show; preserve | none | none | delivery::cli | src/cli/show.rs | retain |
| src/cli/sources.rs | comemory::cli::sources; preserve | none | none | delivery::cli | src/cli/sources.rs | retain |
| src/cli/stats.rs | comemory::cli::stats; preserve | none | none | delivery::cli | src/cli/stats.rs | retain |
| src/cli/sync.rs | comemory::cli::sync; preserve | none | none | delivery::cli | src/cli/sync.rs | retain |
| src/cli/sync_render.rs | comemory::cli::sync_render; preserve | src/cli/tests/sync_render.rs | none | delivery::cli | src/cli/sync_render.rs | retain |
| src/cli/tune.rs | comemory::cli::tune; preserve | none | none | delivery::cli | src/cli/tune.rs | retain |
| src/cli/unindex.rs | comemory::cli::unindex; preserve | none | none | delivery::cli | src/cli/unindex.rs | retain |
| src/cli/upgrade.rs | comemory::cli::upgrade; preserve | none | none | delivery::cli | src/cli/upgrade.rs | retain |
| src/cli/watch.rs | comemory::cli::watch; preserve | none | none | delivery::cli | src/cli/watch.rs | retain |
| src/config.rs | comemory::config; preserve | none | none | shared::config | src/config.rs | retain |
| src/config/defaults.rs | private | src/config/tests/defaults.rs | none | shared::config | src/config/defaults.rs | retain |
| src/config/env.rs | comemory::config::env; preserve | src/config/tests/env.rs; src/config/tests/env_2.rs | none | shared::config | src/config/env.rs | retain |
| src/config/file.rs | comemory::config::file; preserve | src/config/tests/file.rs; src/config/tests/file_2.rs | none | shared::config | src/config/file.rs | retain |
| src/config/learning.rs | comemory::config::learning; preserve | src/config/tests/learning.rs | none | shared::config | src/config/learning.rs | retain |
| src/config/observations.rs | comemory::config::observations; preserve | none | none | shared::config | src/config/observations.rs | retain |
| src/config/patch.rs | comemory::config::patch; preserve | src/config/tests/patch.rs | none | shared::config | src/config/patch.rs | retain |
| src/config/paths.rs | comemory::config::paths; preserve | src/config/tests/paths.rs | none | shared::config | src/config/paths.rs | retain |
| src/config/retrieval.rs | comemory::config::retrieval; preserve | src/config/tests/retrieval.rs | none | shared::config | src/config/retrieval.rs | retain |
| src/config/sync.rs | comemory::config::sync; preserve | src/config/tests/sync.rs | none | shared::config | src/config/sync.rs | retain |
| src/config/validate.rs | private | src/config/tests/validate.rs | none | shared::config | src/config/validate.rs | retain |
| src/domains.rs | comemory::domains; preserve | none | none | shared::root | src/domains.rs | retain |
| src/domains/capture.rs | comemory::domains::capture; crate-root-alias | none | none | domains::capture | src/domains/capture.rs | retain |
| src/domains/capture/candidates.rs | comemory::domains::capture::candidates; crate-root-alias | src/domains/capture/tests/candidates.rs | none | domains::capture | src/domains/capture/candidates.rs | retain |
| src/domains/capture/claude_code.rs | comemory::domains::capture::claude_code; crate-root-alias | src/domains/capture/tests/claude_code.rs | none | domains::capture | src/domains/capture/claude_code.rs | retain |
| src/domains/capture/client.rs | comemory::domains::capture::client; crate-root-alias | none | none | domains::capture | src/domains/capture/client.rs | retain |
| src/domains/capture/distill.rs | comemory::domains::capture::distill; crate-root-alias | src/domains/capture/tests/distill.rs | none | domains::capture | src/domains/capture/distill.rs | retain |
| src/domains/capture/explicit_save.rs | comemory::domains::capture::explicit_save; crate-root-alias | src/domains/capture/tests/explicit_save.rs | none | domains::capture | src/domains/capture/explicit_save.rs | retain |
| src/domains/capture/hook.rs | comemory::domains::capture::hook; crate-root-alias | src/domains/capture/tests/hook.rs | none | domains::capture | src/domains/capture/hook.rs | retain |
| src/domains/capture/receipt.rs | comemory::domains::capture::receipt; crate-root-alias | none | none | domains::capture | src/domains/capture/receipt.rs | retain |
| src/domains/capture/redact.rs | comemory::domains::capture::redact; crate-root-alias | src/domains/capture/tests/redact.rs | src/domains/capture/rules.toml | domains::capture | src/domains/capture/redact.rs | retain |
| src/domains/capture/run.rs | comemory::domains::capture::run; crate-root-alias | src/domains/capture/tests/run.rs | none | domains::capture | src/domains/capture/run.rs | retain |
| src/domains/code.rs | comemory::domains::code; preserve | none | none | domains::code | src/domains/code.rs | retain |
| src/domains/code/ast.rs | comemory::domains::code::ast; crate-root-alias | none | none | domains::code | src/domains/code/ast.rs | retain |
| src/domains/code/ast/chunk.rs | comemory::domains::code::ast::chunk; crate-root-alias | src/domains/code/ast/tests/chunk.rs | none | domains::code | src/domains/code/ast/chunk.rs | retain |
| src/domains/code/ast/extractor.rs | comemory::domains::code::ast::extractor; crate-root-alias | src/domains/code/ast/tests/extractor.rs | none | domains::code | src/domains/code/ast/extractor.rs | retain |
| src/domains/code/ast/languages.rs | comemory::domains::code::ast::languages; crate-root-alias | src/domains/code/ast/tests/languages.rs | none | domains::code | src/domains/code/ast/languages.rs | retain |
| src/domains/code/ast/pattern.rs | comemory::domains::code::ast::pattern; crate-root-alias | src/domains/code/ast/tests/pattern.rs | none | domains::code | src/domains/code/ast/pattern.rs | retain |
| src/domains/code/ast/pattern_cache.rs | private | src/domains/code/ast/tests/pattern_cache.rs | none | domains::code | src/domains/code/ast/pattern_cache.rs | retain |
| src/domains/code/git_utils.rs | comemory::domains::code::git_utils; crate-root-alias | src/domains/code/tests/git_utils.rs | none | domains::code | src/domains/code/git_utils.rs | retain |
| src/domains/code/hooks.rs | comemory::domains::code::hooks; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/code/tests/hooks.rs | none | domains::code | src/domains/code/hooks.rs | retain |
| src/domains/code/index_code.rs | comemory::domains::code::index_code; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/code/tests/index_code.rs | none | domains::code | src/domains/code/index_code.rs | retain |
| src/domains/code/index_code/walk.rs | comemory::domains::code::index_code::walk; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/code/index_code/tests/walk.rs | none | domains::code | src/domains/code/index_code/walk.rs | retain |
| src/domains/code/index_runs.rs | comemory::domains::code::index_runs; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/code/tests/index_runs.rs | none | domains::code | src/domains/code/index_runs.rs | retain |
| src/domains/code/ingest_code.rs | comemory::domains::code::ingest_code; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/code/tests/ingest_code.rs | none | domains::code | src/domains/code/ingest_code.rs | retain |
| src/domains/code/install_hooks.rs | comemory::domains::code::install_hooks; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::code | src/domains/code/install_hooks.rs | retain |
| src/domains/code/pattern_search.rs | comemory::domains::code::pattern_search; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/code/tests/ast.rs | none | domains::code | src/domains/code/pattern_search.rs | retain |
| src/domains/code/reindex_policy.rs | comemory::domains::code::reindex_policy; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::code | src/domains/code/reindex_policy.rs | retain |
| src/domains/code/repo_admin.rs | comemory::domains::code::repo_admin; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/code/tests/repo_admin.rs | none | domains::code | src/domains/code/repo_admin.rs | retain |
| src/domains/code/repos.rs | comemory::domains::code::repos; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/code/tests/repos.rs | none | domains::code | src/domains/code/repos.rs | retain |
| src/domains/code/repos/git_state.rs | comemory::domains::code::repos::git_state; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/code/repos/tests/git_state.rs | none | domains::code | src/domains/code/repos/git_state.rs | retain |
| src/domains/documents.rs | comemory::domains::documents; preserve | none | none | domains::documents | src/domains/documents.rs | retain |
| src/domains/documents/document.rs | comemory::domains::documents::document; crate-root-alias | none | none | domains::documents | src/domains/documents/document.rs | retain |
| src/domains/documents/document/chunk.rs | comemory::domains::documents::document::chunk; crate-root-alias | src/domains/documents/document/tests/chunk.rs | none | domains::documents | src/domains/documents/document/chunk.rs | retain |
| src/domains/documents/document/delimited.rs | comemory::domains::documents::document::delimited; crate-root-alias | src/domains/documents/document/tests/delimited.rs | none | domains::documents | src/domains/documents/document/delimited.rs | retain |
| src/domains/documents/document/extract.rs | comemory::domains::documents::document::extract; crate-root-alias | src/domains/documents/document/tests/extract.rs | none | domains::documents | src/domains/documents/document/extract.rs | retain |
| src/domains/documents/document/fingerprint.rs | comemory::domains::documents::document::fingerprint; crate-root-alias | src/domains/documents/document/tests/fingerprint.rs | none | domains::documents | src/domains/documents/document/fingerprint.rs | retain |
| src/domains/documents/document/html.rs | comemory::domains::documents::document::html; crate-root-alias | src/domains/documents/document/tests/html.rs | none | domains::documents | src/domains/documents/document/html.rs | retain |
| src/domains/documents/document/writer.rs | comemory::domains::documents::document::writer; crate-root-alias | src/domains/documents/document/tests/writer.rs | none | domains::documents | src/domains/documents/document/writer.rs | retain |
| src/domains/documents/index.rs | comemory::domains::documents::index; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/documents/tests/index.rs | none | domains::documents | src/domains/documents/index.rs | retain |
| src/domains/documents/source.rs | comemory::domains::documents::source; crate-root-alias | none | none | domains::documents | src/domains/documents/source.rs | retain |
| src/domains/documents/source/classify.rs | comemory::domains::documents::source::classify; crate-root-alias | src/domains/documents/source/tests/classify.rs | none | domains::documents | src/domains/documents/source/classify.rs | retain |
| src/domains/documents/source/discover.rs | comemory::domains::documents::source::discover; crate-root-alias | src/domains/documents/source/tests/discover.rs | none | domains::documents | src/domains/documents/source/discover.rs | retain |
| src/domains/documents/source/mirror.rs | comemory::domains::documents::source::mirror; crate-root-alias | src/domains/documents/source/tests/mirror.rs | none | domains::documents | src/domains/documents/source/mirror.rs | retain |
| src/domains/documents/source/registry.rs | comemory::domains::documents::source::registry; crate-root-alias | src/domains/documents/source/tests/registry.rs | none | domains::documents | src/domains/documents/source/registry.rs | retain |
| src/domains/documents/sources.rs | comemory::domains::documents::sources; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::documents | src/domains/documents/sources.rs | retain |
| src/domains/documents/unindex.rs | comemory::domains::documents::unindex; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::documents | src/domains/documents/unindex.rs | retain |
| src/domains/graph.rs | comemory::domains::graph; crate-root-alias | none | none | domains::graph | src/domains/graph.rs | retain |
| src/domains/graph/coactivate.rs | comemory::domains::graph::coactivate; crate-root-alias | src/domains/graph/tests/coactivate.rs | none | domains::graph | src/domains/graph/coactivate.rs | retain |
| src/domains/graph/cochange.rs | comemory::domains::graph::cochange; crate-root-alias | src/domains/graph/tests/cochange.rs | none | domains::graph | src/domains/graph/cochange.rs | retain |
| src/domains/graph/code_graph.rs | comemory::domains::graph::code_graph; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/graph/tests/code_graph.rs | none | domains::graph | src/domains/graph/code_graph.rs | retain |
| src/domains/graph/cross_link.rs | comemory::domains::graph::cross_link; crate-root-alias | src/domains/graph/tests/cross_link.rs | none | domains::graph | src/domains/graph/cross_link.rs | retain |
| src/domains/graph/derived.rs | comemory::domains::graph::derived; crate-root-alias | src/domains/graph/tests/derived.rs | none | domains::graph | src/domains/graph/derived.rs | retain |
| src/domains/graph/doc_link.rs | comemory::domains::graph::doc_link; crate-root-alias | src/domains/graph/tests/doc_link.rs | none | domains::graph | src/domains/graph/doc_link.rs | retain |
| src/domains/graph/edges.rs | comemory::domains::graph::edges; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::graph | src/domains/graph/edges.rs | retain |
| src/domains/graph/edges_result.rs | comemory::domains::graph::edges_result; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/graph/tests/edges_result.rs | none | domains::graph | src/domains/graph/edges_result.rs | retain |
| src/domains/graph/graph_nodes.rs | comemory::domains::graph::graph_nodes; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/graph/tests/graph_nodes.rs | none | domains::graph | src/domains/graph/graph_nodes.rs | retain |
| src/domains/graph/graph_recompute.rs | comemory::domains::graph::graph_recompute; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/graph/tests/graph_recompute.rs | none | domains::graph | src/domains/graph/graph_recompute.rs | retain |
| src/domains/graph/imports.rs | comemory::domains::graph::imports; crate-root-alias | src/domains/graph/tests/imports.rs | none | domains::graph | src/domains/graph/imports.rs | retain |
| src/domains/graph/materialize.rs | comemory::domains::graph::materialize; crate-root-alias | src/domains/graph/tests/materialize.rs | none | domains::graph | src/domains/graph/materialize.rs | retain |
| src/domains/graph/memory_rank.rs | comemory::domains::graph::memory_rank; crate-root-alias | src/domains/graph/tests/memory_rank.rs | none | domains::graph | src/domains/graph/memory_rank.rs | retain |
| src/domains/graph/neighbors.rs | comemory::domains::graph::neighbors; crate-root-alias | src/domains/graph/tests/neighbors.rs | none | domains::graph | src/domains/graph/neighbors.rs | retain |
| src/domains/graph/nodes.rs | comemory::domains::graph::nodes; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::graph | src/domains/graph/nodes.rs | retain |
| src/domains/graph/pagerank.rs | comemory::domains::graph::pagerank; crate-root-alias | src/domains/graph/tests/pagerank.rs | none | domains::graph | src/domains/graph/pagerank.rs | retain |
| src/domains/graph/query.rs | comemory::domains::graph::query; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/graph/tests/query.rs | none | domains::graph | src/domains/graph/query.rs | retain |
| src/domains/graph/search_edit.rs | private | src/domains/graph/tests/search_edit.rs | none | domains::graph | src/domains/graph/search_edit.rs | retain |
| src/domains/graph/view.rs | comemory::domains::graph::view; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::graph | src/domains/graph/view.rs | retain |
| src/domains/integrations.rs | comemory::domains::integrations; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::integrations | src/domains/integrations.rs | retain |
| src/domains/integrations/install.rs | comemory::domains::integrations::install; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/integrations/tests/install.rs | none | domains::integrations | src/domains/integrations/install.rs | retain |
| src/domains/integrations/install/bundle.rs | private | src/domains/integrations/install/tests/bundle.rs | integrations/agent/.claude-plugin/plugin.json; integrations/agent/.codex-plugin/plugin.json; integrations/agent/hooks/comemory-status.sh; integrations/agent/hooks/hooks.json; integrations/agent/hooks/memory-lifecycle.sh; integrations/agent/hooks/project-skills-curate.sh; integrations/agent/hooks/project-skills-index.sh; integrations/agent/hooks/scope.sh; integrations/agent/hooks/session-end.sh; integrations/agent/hooks/session-start.sh; integrations/agent/hooks/skill-use.sh; integrations/agent/lib/project-skills-commands.sh; integrations/agent/lib/project-skills-curation.sh; integrations/agent/lib/project-skills-foundation.sh; integrations/agent/lib/project-skills.sh; integrations/agent/lib/recall.sh; integrations/agent/lib/repo-scope.sh; integrations/agent/lib/shell-input.sh; integrations/agent/skills/agent-memory/SKILL.md; integrations/agent/skills/agent-memory/scripts/comemory.sh; integrations/agent/skills/memory-bootstrap/SKILL.md; integrations/agent/skills/project-skills/SKILL.md; integrations/agent/skills/project-skills/scripts/skills.sh | domains::integrations | src/domains/integrations/install/bundle.rs | retain |
| src/domains/integrations/setup.rs | comemory::domains::integrations::setup; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/integrations/tests/setup.rs | none | domains::integrations | src/domains/integrations/setup.rs | retain |
| src/domains/integrations/setup/apply.rs | comemory::domains::integrations::setup::apply; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/integrations/setup/tests/apply.rs | none | domains::integrations | src/domains/integrations/setup/apply.rs | retain |
| src/domains/integrations/setup/detect.rs | comemory::domains::integrations::setup::detect; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/integrations/setup/tests/detect.rs | none | domains::integrations | src/domains/integrations/setup/detect.rs | retain |
| src/domains/integrations/setup/plan.rs | comemory::domains::integrations::setup::plan; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/integrations/setup/tests/plan.rs | none | domains::integrations | src/domains/integrations/setup/plan.rs | retain |
| src/domains/learning.rs | comemory::domains::learning; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::learning | src/domains/learning.rs | retain |
| src/domains/learning/bandit.rs | comemory::domains::learning::bandit; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/learning/tests/bandit.rs | none | domains::learning | src/domains/learning/bandit.rs | retain |
| src/domains/learning/code_feedback.rs | comemory::domains::learning::code_feedback; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/learning/tests/code_feedback.rs | none | domains::learning | src/domains/learning/code_feedback.rs | retain |
| src/domains/learning/console.rs | comemory::domains::learning::console; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/learning/tests/console.rs | none | domains::learning | src/domains/learning/console.rs | retain |
| src/domains/learning/benchmark.rs | comemory::domains::learning::benchmark; preserve | src/domains/learning/tests/benchmark.rs | none | domains::learning | src/domains/learning/benchmark.rs | retain |
| src/domains/learning/evaluation/benchmark_arm.rs | comemory::domains::learning::evaluation::benchmark_arm; crate-root-alias | src/domains/learning/evaluation/tests/benchmark_arm.rs | none | domains::learning | src/domains/learning/evaluation/benchmark_arm.rs | retain |
| src/domains/learning/evaluation/benchmark_arm_report.rs | comemory::domains::learning::evaluation::benchmark_arm_report; crate-root-alias | src/domains/learning/evaluation/tests/benchmark_arm_report.rs | none | domains::learning | src/domains/learning/evaluation/benchmark_arm_report.rs | retain |
| src/domains/learning/evaluation/benchmark_metrics.rs | comemory::domains::learning::evaluation::benchmark_metrics; crate-root-alias | src/domains/learning/evaluation/tests/benchmark_metrics.rs | none | domains::learning | src/domains/learning/evaluation/benchmark_metrics.rs | retain |
| src/domains/learning/evaluation/benchmark_observe.rs | comemory::domains::learning::evaluation::benchmark_observe; crate-root-alias | none | none | domains::learning | src/domains/learning/evaluation/benchmark_observe.rs | retain |
| src/domains/learning/evaluation/benchmark_report.rs | comemory::domains::learning::evaluation::benchmark_report; crate-root-alias | none | none | domains::learning | src/domains/learning/evaluation/benchmark_report.rs | retain |
| src/domains/learning/evaluation/benchmark_runner.rs | comemory::domains::learning::evaluation::benchmark_runner; crate-root-alias | src/domains/learning/evaluation/tests/benchmark_runner.rs | none | domains::learning | src/domains/learning/evaluation/benchmark_runner.rs | retain |
| src/domains/learning/evaluation/benchmark_set.rs | comemory::domains::learning::evaluation::benchmark_set; crate-root-alias | src/domains/learning/evaluation/tests/benchmark_set.rs | none | domains::learning | src/domains/learning/evaluation/benchmark_set.rs | retain |
| src/domains/learning/evaluation/candidate_facts.rs | comemory::domains::learning::evaluation::candidate_facts; crate-root-alias | none | none | domains::learning | src/domains/learning/evaluation/candidate_facts.rs | retain |
| src/domains/learning/evaluation/candidate_identity.rs | comemory::domains::learning::evaluation::candidate_identity; crate-root-alias | src/domains/learning/evaluation/tests/candidate_identity.rs | none | domains::learning | src/domains/learning/evaluation/candidate_identity.rs | retain |
| src/domains/learning/evaluation/candidate_observation.rs | comemory::domains::learning::evaluation::candidate_observation; crate-root-alias | src/domains/learning/evaluation/tests/candidate_observation.rs | none | domains::learning | src/domains/learning/evaluation/candidate_observation.rs | retain |
| src/domains/learning/evaluation/dataset_build.rs | comemory::domains::learning::evaluation::dataset_build; crate-root-alias | none | none | domains::learning | src/domains/learning/evaluation/dataset_build.rs | retain |
| src/domains/learning/evaluation/dataset_dedup.rs | comemory::domains::learning::evaluation::dataset_dedup; crate-root-alias | src/domains/learning/evaluation/tests/dataset_dedup.rs | none | domains::learning | src/domains/learning/evaluation/dataset_dedup.rs | retain |
| src/domains/learning/evaluation/dataset_files.rs | comemory::domains::learning::evaluation::dataset_files; crate-root-alias | none | none | domains::learning | src/domains/learning/evaluation/dataset_files.rs | retain |
| src/domains/learning/evaluation/dataset_manifest.rs | comemory::domains::learning::evaluation::dataset_manifest; crate-root-alias | src/domains/learning/evaluation/tests/dataset_manifest.rs | none | domains::learning | src/domains/learning/evaluation/dataset_manifest.rs | retain |
| src/domains/learning/evaluation/dataset_record.rs | comemory::domains::learning::evaluation::dataset_record; crate-root-alias | none | none | domains::learning | src/domains/learning/evaluation/dataset_record.rs | retain |
| src/domains/learning/evaluation/dataset_rows.rs | comemory::domains::learning::evaluation::dataset_rows; crate-root-alias | src/domains/learning/evaluation/tests/dataset_rows.rs | none | domains::learning | src/domains/learning/evaluation/dataset_rows.rs | retain |
| src/domains/learning/evaluation/dataset_select.rs | comemory::domains::learning::evaluation::dataset_select; crate-root-alias | none | none | domains::learning | src/domains/learning/evaluation/dataset_select.rs | retain |
| src/domains/learning/evaluation/dataset_split.rs | comemory::domains::learning::evaluation::dataset_split; crate-root-alias | src/domains/learning/evaluation/tests/dataset_split.rs | none | domains::learning | src/domains/learning/evaluation/dataset_split.rs | retain |
| src/domains/learning/evaluation/judgment.rs | comemory::domains::learning::evaluation::judgment; crate-root-alias | src/domains/learning/evaluation/tests/judgment.rs | none | domains::learning | src/domains/learning/evaluation/judgment.rs | retain |
| src/domains/learning/evaluation/run_environment.rs | comemory::domains::learning::evaluation::run_environment; crate-root-alias | src/domains/learning/evaluation/tests/run_environment.rs | none | domains::learning | src/domains/learning/evaluation/run_environment.rs | retain |
| src/domains/learning/evaluation/task_filters.rs | comemory::domains::learning::evaluation::task_filters; crate-root-alias | none | none | domains::learning | src/domains/learning/evaluation/task_filters.rs | retain |
| src/domains/learning/eval.rs | comemory::domains::learning::eval; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/learning/tests/eval.rs | none | domains::learning | src/domains/learning/eval.rs | retain |
| src/domains/learning/evaluation.rs | comemory::domains::learning::evaluation; crate-root-alias | none | none | domains::learning | src/domains/learning/evaluation.rs | retain |
| src/domains/learning/evaluation/bandit.rs | comemory::domains::learning::evaluation::bandit; crate-root-alias | src/domains/learning/evaluation/tests/bandit.rs | none | domains::learning | src/domains/learning/evaluation/bandit.rs | retain |
| src/domains/learning/evaluation/bandit_rng.rs | comemory::domains::learning::evaluation::bandit_rng; crate-root-alias | src/domains/learning/evaluation/tests/bandit_rng.rs | none | domains::learning | src/domains/learning/evaluation/bandit_rng.rs | retain |
| src/domains/learning/evaluation/golden.rs | comemory::domains::learning::evaluation::golden; crate-root-alias | src/domains/learning/evaluation/tests/golden.rs | none | domains::learning | src/domains/learning/evaluation/golden.rs | retain |
| src/domains/learning/evaluation/metrics.rs | comemory::domains::learning::evaluation::metrics; crate-root-alias | src/domains/learning/evaluation/tests/metrics.rs | none | domains::learning | src/domains/learning/evaluation/metrics.rs | retain |
| src/domains/learning/evaluation/mine.rs | comemory::domains::learning::evaluation::mine; crate-root-alias | src/domains/learning/evaluation/tests/mine.rs | none | domains::learning | src/domains/learning/evaluation/mine.rs | retain |
| src/domains/learning/evaluation/runner.rs | comemory::domains::learning::evaluation::runner; crate-root-alias | src/domains/learning/evaluation/tests/runner.rs | none | domains::learning | src/domains/learning/evaluation/runner.rs | retain |
| src/domains/learning/evaluation/tune.rs | comemory::domains::learning::evaluation::tune; crate-root-alias | src/domains/learning/evaluation/tests/tune.rs | none | domains::learning | src/domains/learning/evaluation/tune.rs | retain |
| src/domains/learning/evaluation/tune_sample.rs | comemory::domains::learning::evaluation::tune_sample; crate-root-alias | src/domains/learning/evaluation/tests/tune_sample.rs | none | domains::learning | src/domains/learning/evaluation/tune_sample.rs | retain |
| src/domains/learning/feedback.rs | comemory::domains::learning::feedback; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/learning/tests/feedback.rs | none | domains::learning | src/domains/learning/feedback.rs | retain |
| src/domains/learning/feedback_tracking.rs | comemory::domains::learning::feedback_tracking; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/learning/tests/feedback_tracking.rs | none | domains::learning | src/domains/learning/feedback_tracking.rs | retain |
| src/domains/learning/dataset_export.rs | comemory::domains::learning::dataset_export; preserve | src/domains/learning/tests/dataset_export.rs | none | domains::learning | src/domains/learning/dataset_export.rs | retain |
| src/domains/learning/judge.rs | comemory::domains::learning::judge; preserve | src/domains/learning/tests/judge.rs | none | domains::learning | src/domains/learning/judge.rs | retain |
| src/domains/learning/learning_proposals.rs | comemory::domains::learning::learning_proposals; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/learning/tests/learning_proposals.rs | none | domains::learning | src/domains/learning/learning_proposals.rs | retain |
| src/domains/learning/mine.rs | comemory::domains::learning::mine; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::learning | src/domains/learning/mine.rs | retain |
| src/domains/learning/observation_capture.rs | comemory::domains::learning::observation_capture; preserve | src/domains/learning/tests/observation_capture.rs | none | domains::learning | src/domains/learning/observation_capture.rs | retain |
| src/domains/learning/recall_status.rs | comemory::domains::learning::recall_status; preserve | src/domains/learning/tests/recall_status.rs | none | domains::learning | src/domains/learning/recall_status.rs | retain |
| src/domains/learning/telemetry.rs | comemory::domains::learning::telemetry; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/learning/tests/telemetry.rs | none | domains::learning | src/domains/learning/telemetry.rs | retain |
| src/domains/learning/tune.rs | comemory::domains::learning::tune; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/learning/tests/tune.rs | none | domains::learning | src/domains/learning/tune.rs | retain |
| src/domains/maintenance.rs | comemory::domains::maintenance; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::maintenance | src/domains/maintenance.rs | retain |
| src/domains/maintenance/consolidate.rs | comemory::domains::maintenance::consolidate; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::maintenance | src/domains/maintenance/consolidate.rs | retain |
| src/domains/maintenance/consolidation.rs | comemory::domains::maintenance::consolidation; crate-root-alias | src/domains/maintenance/tests/consolidation.rs | none | domains::maintenance | src/domains/maintenance/consolidation.rs | retain |
| src/domains/maintenance/consolidation/cluster.rs | comemory::domains::maintenance::consolidation::cluster; crate-root-alias | src/domains/maintenance/consolidation/tests/cluster.rs | none | domains::maintenance | src/domains/maintenance/consolidation/cluster.rs | retain |
| src/domains/maintenance/consolidation/keeper.rs | comemory::domains::maintenance::consolidation::keeper; crate-root-alias | src/domains/maintenance/consolidation/tests/keeper.rs | none | domains::maintenance | src/domains/maintenance/consolidation/keeper.rs | retain |
| src/domains/maintenance/consolidation_report.rs | comemory::domains::maintenance::consolidation_report; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::maintenance | src/domains/maintenance/consolidation_report.rs | retain |
| src/domains/maintenance/doctor.rs | comemory::domains::maintenance::doctor; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/maintenance/tests/doctor.rs | none | domains::maintenance | src/domains/maintenance/doctor.rs | retain |
| src/domains/maintenance/doctor/backup.rs | comemory::domains::maintenance::doctor::backup; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::maintenance | src/domains/maintenance/doctor/backup.rs | retain |
| src/domains/maintenance/doctor/checks.rs | comemory::domains::maintenance::doctor::checks; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::maintenance | src/domains/maintenance/doctor/checks.rs | retain |
| src/domains/maintenance/doctor/system.rs | comemory::domains::maintenance::doctor::system; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/maintenance/tests/doctor_system.rs | none | domains::maintenance | src/domains/maintenance/doctor/system.rs | retain |
| src/domains/maintenance/gc.rs | comemory::domains::maintenance::gc; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/maintenance/tests/gc.rs | none | domains::maintenance | src/domains/maintenance/gc.rs | retain |
| src/domains/maintenance/gc_policy.rs | comemory::domains::maintenance::gc_policy; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/maintenance/tests/gc_policy.rs | none | domains::maintenance | src/domains/maintenance/gc_policy.rs | retain |
| src/domains/maintenance/overview.rs | comemory::domains::maintenance::overview; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/maintenance/tests/overview.rs | none | domains::maintenance | src/domains/maintenance/overview.rs | retain |
| src/domains/maintenance/prune.rs | comemory::domains::maintenance::prune; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/maintenance/tests/prune.rs | none | domains::maintenance | src/domains/maintenance/prune.rs | retain |
| src/domains/maintenance/rebuild.rs | comemory::domains::maintenance::rebuild; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/maintenance/tests/rebuild.rs | none | domains::maintenance | src/domains/maintenance/rebuild.rs | retain |
| src/domains/maintenance/rebuild/copy.rs | comemory::domains::maintenance::rebuild::copy; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/maintenance/rebuild/tests/coverage.rs | none | domains::maintenance | src/domains/maintenance/rebuild/copy.rs | retain |
| src/domains/maintenance/reembed.rs | comemory::domains::maintenance::reembed; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/maintenance/tests/reembed.rs | none | domains::maintenance | src/domains/maintenance/reembed.rs | retain |
| src/domains/maintenance/retention.rs | comemory::domains::maintenance::retention; crate-root-alias | none | none | domains::maintenance | src/domains/maintenance/retention.rs | retain |
| src/domains/maintenance/retention/low_value.rs | comemory::domains::maintenance::retention::low_value; crate-root-alias | src/domains/maintenance/retention/tests/low_value.rs | none | domains::maintenance | src/domains/maintenance/retention/low_value.rs | retain |
| src/domains/maintenance/retention/orphans.rs | comemory::domains::maintenance::retention::orphans; crate-root-alias | src/domains/maintenance/retention/tests/orphans.rs | none | domains::maintenance | src/domains/maintenance/retention/orphans.rs | retain |
| src/domains/maintenance/retention/stale_code.rs | comemory::domains::maintenance::retention::stale_code; crate-root-alias | src/domains/maintenance/retention/tests/stale_code.rs | none | domains::maintenance | src/domains/maintenance/retention/stale_code.rs | retain |
| src/domains/maintenance/retention_report.rs | comemory::domains::maintenance::retention_report; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::maintenance | src/domains/maintenance/retention_report.rs | retain |
| src/domains/maintenance/stats.rs | comemory::domains::maintenance::stats; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/maintenance/tests/stats.rs | none | domains::maintenance | src/domains/maintenance/stats.rs | retain |
| src/domains/maintenance/upgrade.rs | comemory::domains::maintenance::upgrade; crate-root-alias | none | none | domains::maintenance | src/domains/maintenance/upgrade.rs | retain |
| src/domains/maintenance/upgrade/channel.rs | comemory::domains::maintenance::upgrade::channel; crate-root-alias | src/domains/maintenance/upgrade/tests/channel.rs | none | domains::maintenance | src/domains/maintenance/upgrade/channel.rs | retain |
| src/domains/maintenance/upgrade/installer.rs | comemory::domains::maintenance::upgrade::installer; crate-root-alias | none | none | domains::maintenance | src/domains/maintenance/upgrade/installer.rs | retain |
| src/domains/maintenance/upgrade/release.rs | comemory::domains::maintenance::upgrade::release; crate-root-alias | none | none | domains::maintenance | src/domains/maintenance/upgrade/release.rs | retain |
| src/domains/maintenance/upgrade/version.rs | comemory::domains::maintenance::upgrade::version; crate-root-alias | src/domains/maintenance/upgrade/tests/version.rs | none | domains::maintenance | src/domains/maintenance/upgrade/version.rs | retain |
| src/domains/memories.rs | comemory::domains::memories; crate-root-alias | none | none | domains::memories | src/domains/memories.rs | retain |
| src/domains/memories/delete.rs | comemory::domains::memories::delete; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::memories | src/domains/memories/delete.rs | retain |
| src/domains/memories/frontmatter.rs | comemory::domains::memories::frontmatter; crate-root-alias | src/domains/memories/tests/frontmatter.rs | none | domains::memories | src/domains/memories/frontmatter.rs | retain |
| src/domains/memories/id.rs | comemory::domains::memories::id; crate-root-alias | src/domains/memories/tests/id.rs | none | domains::memories | src/domains/memories/id.rs | retain |
| src/domains/memories/list.rs | comemory::domains::memories::list; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::memories | src/domains/memories/list.rs | retain |
| src/domains/memories/mirror.rs | comemory::domains::memories::mirror; preserve | src/domains/memories/tests/mirror.rs | none | domains::memories | src/domains/memories/mirror.rs | retain |
| src/domains/memories/nav.rs | private | none | none | domains::memories | src/domains/memories/nav.rs | retain |
| src/domains/memories/prior.rs | comemory::domains::memories::prior; crate-root-alias | src/domains/memories/tests/prior.rs | none | domains::memories | src/domains/memories/prior.rs | retain |
| src/domains/memories/references.rs | comemory::domains::memories::references; crate-root-alias | src/domains/memories/tests/references.rs | none | domains::memories | src/domains/memories/references.rs | retain |
| src/domains/memories/refresh_refs.rs | comemory::domains::memories::refresh_refs; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/memories/tests/refresh_refs.rs | none | domains::memories | src/domains/memories/refresh_refs.rs | retain |
| src/domains/memories/restore.rs | comemory::domains::memories::restore; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/memories/tests/restore.rs | none | domains::memories | src/domains/memories/restore.rs | retain |
| src/domains/memories/save.rs | comemory::domains::memories::save; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/memories/tests/save.rs | none | domains::memories | src/domains/memories/save.rs | retain |
| src/domains/memories/show.rs | comemory::domains::memories::show; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/memories/tests/show.rs | none | domains::memories | src/domains/memories/show.rs | retain |
| src/domains/memories/slug.rs | comemory::domains::memories::slug; crate-root-alias | src/domains/memories/tests/slug.rs | none | domains::memories | src/domains/memories/slug.rs | retain |
| src/domains/memories/store.rs | comemory::domains::memories::store; crate-root-alias | src/domains/memories/tests/store.rs | none | domains::memories | src/domains/memories/store.rs | retain |
| src/domains/memories/trash.rs | comemory::domains::memories::trash; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/memories/tests/trash.rs | none | domains::memories | src/domains/memories/trash.rs | retain |
| src/domains/memories/update.rs | comemory::domains::memories::update; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/memories/tests/update.rs | none | domains::memories | src/domains/memories/update.rs | retain |
| src/domains/retrieval.rs | comemory::domains::retrieval; crate-root-alias | none | none | domains::retrieval | src/domains/retrieval.rs | retain |
| src/domains/retrieval/bundle.rs | comemory::domains::retrieval::bundle; crate-root-alias | src/domains/retrieval/tests/bundle.rs | none | domains::retrieval | src/domains/retrieval/bundle.rs | retain |
| src/domains/retrieval/code_prior.rs | comemory::domains::retrieval::code_prior; crate-root-alias | src/domains/retrieval/tests/code_prior.rs | none | domains::retrieval | src/domains/retrieval/code_prior.rs | retain |
| src/domains/retrieval/code_ref_collect.rs | comemory::domains::retrieval::code_ref_collect; crate-root-alias | src/domains/retrieval/tests/code_ref_collect.rs | none | domains::retrieval | src/domains/retrieval/code_ref_collect.rs | retain |
| src/domains/retrieval/code_ref_fetch.rs | comemory::domains::retrieval::code_ref_fetch; crate-root-alias | src/domains/retrieval/tests/code_ref_fetch.rs | none | domains::retrieval | src/domains/retrieval/code_ref_fetch.rs | retain |
| src/domains/retrieval/code_ref_status.rs | comemory::domains::retrieval::code_ref_status; crate-root-alias | src/domains/retrieval/tests/code_ref_status.rs | none | domains::retrieval | src/domains/retrieval/code_ref_status.rs | retain |
| src/domains/retrieval/code_rerank.rs | comemory::domains::retrieval::code_rerank; crate-root-alias | src/domains/retrieval/tests/code_rerank.rs; src/domains/retrieval/tests/code_rerank_2.rs | none | domains::retrieval | src/domains/retrieval/code_rerank.rs | retain |
| src/domains/retrieval/code_route.rs | comemory::domains::retrieval::code_route; crate-root-alias | src/domains/retrieval/tests/code_route.rs | none | domains::retrieval | src/domains/retrieval/code_route.rs | retain |
| src/domains/retrieval/code_search.rs | comemory::domains::retrieval::code_search; crate-root-alias | src/domains/retrieval/tests/code_search.rs | none | domains::retrieval | src/domains/retrieval/code_search.rs | retain |
| src/domains/retrieval/code_search_result.rs | comemory::domains::retrieval::code_search_result; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::retrieval | src/domains/retrieval/code_search_result.rs | retain |
| src/domains/retrieval/config_retrieval.rs | comemory::domains::retrieval::config_retrieval; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/retrieval/tests/config_retrieval.rs | none | domains::retrieval | src/domains/retrieval/config_retrieval.rs | retain |
| src/domains/retrieval/context.rs | comemory::domains::retrieval::context; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::retrieval | src/domains/retrieval/context.rs | retain |
| src/domains/retrieval/context_result.rs | comemory::domains::retrieval::context_result; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::retrieval | src/domains/retrieval/context_result.rs | retain |
| src/domains/retrieval/diversify.rs | comemory::domains::retrieval::diversify; crate-root-alias | src/domains/retrieval/tests/diversify.rs | none | domains::retrieval | src/domains/retrieval/diversify.rs | retain |
| src/domains/retrieval/doc_route.rs | comemory::domains::retrieval::doc_route; crate-root-alias | src/domains/retrieval/tests/doc_route.rs | none | domains::retrieval | src/domains/retrieval/doc_route.rs | retain |
| src/domains/retrieval/explain.rs | comemory::domains::retrieval::explain; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/retrieval/tests/explain.rs | none | domains::retrieval | src/domains/retrieval/explain.rs | retain |
| src/domains/retrieval/find.rs | comemory::domains::retrieval::find; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/retrieval/tests/find.rs | none | domains::retrieval | src/domains/retrieval/find.rs | retain |
| src/domains/retrieval/fuse.rs | comemory::domains::retrieval::fuse; crate-root-alias | src/domains/retrieval/tests/fuse.rs | none | domains::retrieval | src/domains/retrieval/fuse.rs | retain |
| src/domains/retrieval/graph_route.rs | comemory::domains::retrieval::graph_route; crate-root-alias | src/domains/retrieval/tests/graph_route.rs | none | domains::retrieval | src/domains/retrieval/graph_route.rs | retain |
| src/domains/retrieval/pipeline.rs | comemory::domains::retrieval::pipeline; crate-root-alias | src/domains/retrieval/tests/pipeline.rs | none | domains::retrieval | src/domains/retrieval/pipeline.rs | retain |
| src/domains/retrieval/rerank.rs | comemory::domains::retrieval::rerank; crate-root-alias | src/domains/retrieval/tests/rerank.rs | none | domains::retrieval | src/domains/retrieval/rerank.rs | retain |
| src/domains/retrieval/router.rs | comemory::domains::retrieval::router; crate-root-alias | src/domains/retrieval/tests/router.rs; src/domains/retrieval/tests/router_2.rs | none | domains::retrieval | src/domains/retrieval/router.rs | retain |
| src/domains/retrieval/scope.rs | comemory::domains::retrieval::scope; crate-root-alias | src/domains/retrieval/tests/scope.rs | none | domains::retrieval | src/domains/retrieval/scope.rs | retain |
| src/domains/retrieval/score.rs | comemory::domains::retrieval::score; crate-root-alias | src/domains/retrieval/tests/score.rs | none | domains::retrieval | src/domains/retrieval/score.rs | retain |
| src/domains/retrieval/search.rs | comemory::domains::retrieval::search; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::retrieval | src/domains/retrieval/search.rs | retain |
| src/domains/retrieval/search_code.rs | comemory::domains::retrieval::search_code; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::retrieval | src/domains/retrieval/search_code.rs | retain |
| src/domains/retrieval/search_result.rs | comemory::domains::retrieval::search_result; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/retrieval/tests/search_result.rs | none | domains::retrieval | src/domains/retrieval/search_result.rs | retain |
| src/domains/retrieval/suggest.rs | comemory::domains::retrieval::suggest; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/retrieval/tests/suggest.rs | none | domains::retrieval | src/domains/retrieval/suggest.rs | retain |
| src/domains/retrieval/unified.rs | comemory::domains::retrieval::unified; crate-root-alias | src/domains/retrieval/tests/unified.rs | none | domains::retrieval | src/domains/retrieval/unified.rs | retain |
| src/domains/retrieval/unified/fuse_domains.rs | comemory::domains::retrieval::unified::fuse_domains; crate-root-alias | none | none | domains::retrieval | src/domains/retrieval/unified/fuse_domains.rs | retain |
| src/domains/sync.rs | comemory::domains::sync; crate-root-alias | none | none | domains::sync | src/domains/sync.rs | retain |
| src/domains/sync/auth_file.rs | comemory::domains::sync::auth_file; crate-root-alias | src/domains/sync/tests/auth_file.rs | none | domains::sync | src/domains/sync/auth_file.rs | retain |
| src/domains/sync/client.rs | comemory::domains::sync::client; crate-root-alias | src/domains/sync/tests/client.rs; src/domains/sync/tests/client_https.rs | none | domains::sync | src/domains/sync/client.rs | retain |
| src/domains/sync/client_code.rs | comemory::domains::sync::client_code; crate-root-alias | none | none | domains::sync | src/domains/sync/client_code.rs | retain |
| src/domains/sync/cloud.rs | comemory::domains::sync::cloud; crate-root-alias | none | none | domains::sync | src/domains/sync/cloud.rs | retain |
| src/domains/sync/cloud/api_url.rs | comemory::domains::sync::cloud::api_url; crate-root-alias | src/domains/sync/cloud/tests/api_url.rs | none | domains::sync | src/domains/sync/cloud/api_url.rs | retain |
| src/domains/sync/cloud/device.rs | comemory::domains::sync::cloud::device; crate-root-alias | src/domains/sync/cloud/tests/device.rs | none | domains::sync | src/domains/sync/cloud/device.rs | retain |
| src/domains/sync/code.rs | comemory::domains::sync::code; crate-root-alias | src/domains/sync/tests/code.rs | none | domains::sync | src/domains/sync/code.rs | retain |
| src/domains/sync/code_plan.rs | comemory::domains::sync::code_plan; crate-root-alias | src/domains/sync/tests/code_plan.rs | none | domains::sync | src/domains/sync/code_plan.rs | retain |
| src/domains/sync/daemon.rs | comemory::domains::sync::daemon; crate-root-alias | src/domains/sync/tests/daemon.rs | none | domains::sync | src/domains/sync/daemon.rs | retain |
| src/domains/sync/daemon_templates.rs | comemory::domains::sync::daemon_templates; crate-root-alias | none | none | domains::sync | src/domains/sync/daemon_templates.rs | retain |
| src/domains/sync/daemon_unit.rs | comemory::domains::sync::daemon_unit; crate-root-alias | none | none | domains::sync | src/domains/sync/daemon_unit.rs | retain |
| src/domains/sync/exchange.rs | comemory::domains::sync::exchange; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/sync/exchange/tests/changes.rs; src/domains/sync/exchange/tests/import.rs | none | domains::sync | src/domains/sync/exchange.rs | retain |
| src/domains/sync/exchange/changes.rs | comemory::domains::sync::exchange::changes; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::sync | src/domains/sync/exchange/changes.rs | retain |
| src/domains/sync/exchange/code_import.rs | comemory::domains::sync::exchange::code_import; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/sync/exchange/tests/code_import.rs | none | domains::sync | src/domains/sync/exchange/code_import.rs | retain |
| src/domains/sync/exchange/code_import_rules.rs | private | none | none | domains::sync | src/domains/sync/exchange/code_import_rules.rs | retain |
| src/domains/sync/exchange/code_import_write.rs | private | none | none | domains::sync | src/domains/sync/exchange/code_import_write.rs | retain |
| src/domains/sync/exchange/code_manifest.rs | comemory::domains::sync::exchange::code_manifest; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::sync | src/domains/sync/exchange/code_manifest.rs | retain |
| src/domains/sync/exchange/code_types.rs | comemory::domains::sync::exchange::code_types; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::sync | src/domains/sync/exchange/code_types.rs | retain |
| src/domains/sync/exchange/import.rs | comemory::domains::sync::exchange::import; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::sync | src/domains/sync/exchange/import.rs | retain |
| src/domains/sync/exchange/import_rules.rs | private | none | none | domains::sync | src/domains/sync/exchange/import_rules.rs | retain |
| src/domains/sync/exchange/import_state.rs | private | none | none | domains::sync | src/domains/sync/exchange/import_state.rs | retain |
| src/domains/sync/exchange/import_write.rs | private | none | none | domains::sync | src/domains/sync/exchange/import_write.rs | retain |
| src/domains/sync/exchange/manifest.rs | comemory::domains::sync::exchange::manifest; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::sync | src/domains/sync/exchange/manifest.rs | retain |
| src/domains/sync/exchange/types.rs | comemory::domains::sync::exchange::types; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | domains::sync | src/domains/sync/exchange/types.rs | retain |
| src/domains/sync/initial.rs | comemory::domains::sync::initial; crate-root-alias | src/domains/sync/tests/initial.rs | none | domains::sync | src/domains/sync/initial.rs | retain |
| src/domains/sync/login.rs | comemory::domains::sync::login; preserve | src/domains/sync/tests/login.rs | none | domains::sync | src/domains/sync/login.rs | retain |
| src/domains/sync/manual.rs | comemory::domains::sync::manual; preserve | src/domains/sync/tests/manual.rs | none | domains::sync | src/domains/sync/manual.rs | retain |
| src/domains/sync/memory_store.rs | comemory::domains::sync::memory_store; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/domains/sync/tests/memory_store.rs | none | domains::sync | src/domains/sync/memory_store.rs | retain |
| src/domains/sync/memory_store/git.rs | private | none | none | domains::sync | src/domains/sync/memory_store/git.rs | retain |
| src/domains/sync/pull.rs | comemory::domains::sync::pull; crate-root-alias | src/domains/sync/tests/pull.rs | none | domains::sync | src/domains/sync/pull.rs | retain |
| src/domains/sync/push.rs | comemory::domains::sync::push; crate-root-alias | src/domains/sync/tests/push.rs | none | domains::sync | src/domains/sync/push.rs | retain |
| src/domains/sync/push_on_save.rs | comemory::domains::sync::push_on_save; crate-root-alias | src/domains/sync/tests/push_on_save.rs | none | domains::sync | src/domains/sync/push_on_save.rs | retain |
| src/domains/sync/redact.rs | comemory::domains::sync::redact; crate-root-alias | src/domains/sync/tests/redact.rs | src/domains/sync/rules.toml | domains::sync | src/domains/sync/redact.rs | retain |
| src/domains/sync/skip_repos.rs | comemory::domains::sync::skip_repos; crate-root-alias | src/domains/sync/tests/skip_repos.rs | none | domains::sync | src/domains/sync/skip_repos.rs | retain |
| src/domains/sync/verify.rs | comemory::domains::sync::verify; crate-root-alias | src/domains/sync/tests/verify.rs | none | domains::sync | src/domains/sync/verify.rs | retain |
| src/domains/sync/watch.rs | comemory::domains::sync::watch; preserve | src/domains/sync/tests/watch.rs | none | domains::sync | src/domains/sync/watch.rs | retain |
| src/errors.rs | comemory::errors; preserve | none | none | shared::root | src/errors.rs | retain |
| src/lib.rs | private | none | none | shared::root | src/lib.rs | retain |
| src/main.rs | private | none | none | shared::root | src/main.rs | retain |
| src/mcp.rs | comemory::mcp; preserve | none | none | delivery::mcp | src/mcp.rs | retain |
| src/mcp/catalog.rs | comemory::mcp::catalog; preserve | src/mcp/tests/catalog.rs | none | delivery::mcp | src/mcp/catalog.rs | retain |
| src/mcp/exec.rs | comemory::mcp::exec; preserve | src/mcp/tests/exec.rs | none | delivery::mcp | src/mcp/exec.rs | retain |
| src/mcp/params.rs | comemory::mcp::params; preserve | src/mcp/tests/params.rs | none | delivery::mcp | src/mcp/params.rs | retain |
| src/mcp/result.rs | comemory::mcp::result; preserve | src/mcp/tests/result.rs | none | delivery::mcp | src/mcp/result.rs | retain |
| src/mcp/scope.rs | comemory::mcp::scope; preserve | src/mcp/tests/scope.rs | none | delivery::mcp | src/mcp/scope.rs | retain |
| src/mcp/server.rs | comemory::mcp::server; preserve | none | none | delivery::mcp | src/mcp/server.rs | retain |
| src/mcp/state.rs | comemory::mcp::state; preserve | src/mcp/tests/state.rs | none | delivery::mcp | src/mcp/state.rs | retain |
| src/mcp/tools_read.rs | comemory::mcp::tools_read; preserve | none | none | delivery::mcp | src/mcp/tools_read.rs | retain |
| src/mcp/tools_write.rs | comemory::mcp::tools_write; preserve | none | none | delivery::mcp | src/mcp/tools_write.rs | retain |
| src/prelude.rs | comemory::prelude; preserve | none | none | shared::root | src/prelude.rs | retain |
| src/serve.rs | comemory::serve; preserve | none | none | delivery::serve | src/serve.rs | retain |
| src/serve/envelope.rs | comemory::serve::envelope; preserve | src/serve/tests/envelope.rs | none | delivery::serve | src/serve/envelope.rs | retain |
| src/serve/jobs.rs | comemory::serve::jobs; preserve | src/serve/tests/jobs.rs | none | delivery::serve | src/serve/jobs.rs | retain |
| src/serve/jobs/events.rs | comemory::serve::jobs::events; preserve | none | none | delivery::serve | src/serve/jobs/events.rs | retain |
| src/serve/jobs/registry.rs | comemory::serve::jobs::registry; preserve | src/serve/jobs/tests/registry.rs | none | delivery::serve | src/serve/jobs/registry.rs | retain |
| src/serve/jobs/worker.rs | comemory::serve::jobs::worker; preserve | src/serve/jobs/tests/worker.rs | none | delivery::serve | src/serve/jobs/worker.rs | retain |
| src/serve/router.rs | comemory::serve::router; preserve | src/serve/tests/router.rs | none | delivery::serve | src/serve/router.rs | retain |
| src/serve/routes.rs | comemory::serve::routes; preserve | none | none | delivery::serve | src/serve/routes.rs | retain |
| src/serve/routes/code.rs | comemory::serve::routes::code; preserve | none | none | delivery::serve | src/serve/routes/code.rs | retain |
| src/serve/routes/config.rs | comemory::serve::routes::config; preserve | src/serve/routes/tests/config.rs | none | delivery::serve | src/serve/routes/config.rs | retain |
| src/serve/routes/find.rs | comemory::serve::routes::find; preserve | none | none | delivery::serve | src/serve/routes/find.rs | retain |
| src/serve/routes/graph.rs | comemory::serve::routes::graph; preserve | none | none | delivery::serve | src/serve/routes/graph.rs | retain |
| src/serve/routes/graph_nodes.rs | comemory::serve::routes::graph_nodes; preserve | src/serve/routes/tests/graph_nodes.rs | none | delivery::serve | src/serve/routes/graph_nodes.rs | retain |
| src/serve/routes/hooks.rs | comemory::serve::routes::hooks; preserve | src/serve/routes/tests/hooks_put.rs | none | delivery::serve | src/serve/routes/hooks.rs | retain |
| src/serve/routes/index_runs.rs | comemory::serve::routes::index_runs; preserve | src/serve/routes/tests/index_runs.rs | none | delivery::serve | src/serve/routes/index_runs.rs | retain |
| src/serve/routes/jobs.rs | comemory::serve::routes::jobs; preserve | src/serve/routes/tests/jobs.rs | none | delivery::serve | src/serve/routes/jobs.rs | retain |
| src/serve/routes/learning.rs | comemory::serve::routes::learning; preserve | src/serve/routes/tests/learning.rs; src/serve/routes/tests/learning_2.rs | none | delivery::serve | src/serve/routes/learning.rs | retain |
| src/serve/routes/learning_console.rs | comemory::serve::routes::learning_console; preserve | src/serve/routes/tests/learning_console.rs | none | delivery::serve | src/serve/routes/learning_console.rs | retain |
| src/serve/routes/maint.rs | comemory::serve::routes::maint; preserve | src/serve/routes/tests/consolidate.rs | none | delivery::serve | src/serve/routes/maint.rs | retain |
| src/serve/routes/maint/admin.rs | comemory::serve::routes::maint::admin; preserve | none | none | delivery::serve | src/serve/routes/maint/admin.rs | retain |
| src/serve/routes/maint/doctor.rs | comemory::serve::routes::maint::doctor; preserve | src/serve/routes/tests/maint_doctor.rs | none | delivery::serve | src/serve/routes/maint/doctor.rs | retain |
| src/serve/routes/maint/gc.rs | comemory::serve::routes::maint::gc; preserve | src/serve/routes/tests/maint_gc.rs | none | delivery::serve | src/serve/routes/maint/gc.rs | retain |
| src/serve/routes/maint/prune.rs | comemory::serve::routes::maint::prune; preserve | src/serve/routes/tests/prune_ids.rs | none | delivery::serve | src/serve/routes/maint/prune.rs | retain |
| src/serve/routes/memories.rs | comemory::serve::routes::memories; preserve | none | none | delivery::serve | src/serve/routes/memories.rs | retain |
| src/serve/routes/memories/edit.rs | comemory::serve::routes::memories::edit; preserve | src/serve/routes/tests/memories_edit.rs | none | delivery::serve | src/serve/routes/memories/edit.rs | retain |
| src/serve/routes/memories/search.rs | comemory::serve::routes::memories::search; preserve | none | none | delivery::serve | src/serve/routes/memories/search.rs | retain |
| src/serve/routes/memories/write.rs | comemory::serve::routes::memories::write; preserve | none | none | delivery::serve | src/serve/routes/memories/write.rs | retain |
| src/serve/routes/memory_stores.rs | comemory::serve::routes::memory_stores; preserve | src/serve/routes/tests/memory_stores.rs | none | delivery::serve | src/serve/routes/memory_stores.rs | retain |
| src/serve/routes/meta.rs | comemory::serve::routes::meta; preserve | none | none | delivery::serve | src/serve/routes/meta.rs | retain |
| src/serve/routes/overview.rs | comemory::serve::routes::overview; preserve | src/serve/routes/tests/overview.rs | none | delivery::serve | src/serve/routes/overview.rs | retain |
| src/serve/routes/repos.rs | comemory::serve::routes::repos; preserve | src/serve/routes/tests/repos.rs | none | delivery::serve | src/serve/routes/repos.rs | retain |
| src/serve/routes/repos_admin.rs | comemory::serve::routes::repos_admin; preserve | src/serve/routes/tests/repos_admin.rs | none | delivery::serve | src/serve/routes/repos_admin.rs | retain |
| src/serve/routes/search.rs | comemory::serve::routes::search; preserve | src/serve/routes/tests/search.rs | none | delivery::serve | src/serve/routes/search.rs | retain |
| src/serve/routes/sources.rs | comemory::serve::routes::sources; preserve | src/serve/routes/tests/sources_path.rs | none | delivery::serve | src/serve/routes/sources.rs | retain |
| src/serve/routes/stats.rs | comemory::serve::routes::stats; preserve | none | none | delivery::serve | src/serve/routes/stats.rs | retain |
| src/serve/routes/sync.rs | comemory::serve::routes::sync; preserve | src/serve/routes/tests/sync.rs; src/serve/routes/tests/sync_code.rs | none | delivery::serve | src/serve/routes/sync.rs | retain |
| src/serve/routes/trash.rs | comemory::serve::routes::trash; preserve | src/serve/routes/tests/trash.rs | none | delivery::serve | src/serve/routes/trash.rs | retain |
| src/serve/scope.rs | comemory::serve::scope; preserve | src/serve/tests/scope.rs | none | delivery::serve | src/serve/scope.rs | retain |
| src/serve/security.rs | comemory::serve::security; preserve | src/serve/tests/security.rs | none | delivery::serve | src/serve/security.rs | retain |
| src/store.rs | comemory::store; preserve | none | none | infrastructure::store | src/store.rs | retain |
| src/store/bandit_arms.rs | comemory::store::bandit_arms; preserve | src/store/tests/bandit_arms.rs | none | infrastructure::store | src/store/bandit_arms.rs | retain |
| src/store/busy.rs | comemory::store::busy; preserve | src/store/tests/busy.rs | none | infrastructure::store | src/store/busy.rs | retain |
| src/store/candidate_judgments.rs | comemory::store::candidate_judgments; preserve | src/store/tests/candidate_judgments.rs | none | infrastructure::store | src/store/candidate_judgments.rs | retain |
| src/store/candidate_dataset.rs | comemory::store::candidate_dataset; preserve | src/store/tests/candidate_dataset.rs | none | infrastructure::store | src/store/candidate_dataset.rs | retain |
| src/store/candidate_observations.rs | comemory::store::candidate_observations; preserve | src/store/tests/candidate_observations.rs | none | infrastructure::store | src/store/candidate_observations.rs | retain |
| src/store/code_feedback.rs | comemory::store::code_feedback; preserve | src/store/tests/code_feedback.rs | none | infrastructure::store | src/store/code_feedback.rs | retain |
| src/store/code_graph_edges.rs | comemory::store::code_graph_edges; preserve | src/store/tests/code_graph_edges.rs | none | infrastructure::store | src/store/code_graph_edges.rs | retain |
| src/store/code_graph_nodes.rs | comemory::store::code_graph_nodes; preserve | src/store/tests/code_graph_nodes.rs | none | infrastructure::store | src/store/code_graph_nodes.rs | retain |
| src/store/code_ref.rs | comemory::store::code_ref; preserve | src/store/tests/code_ref.rs | none | infrastructure::store | src/store/code_ref.rs | retain |
| src/store/code_text.rs | comemory::store::code_text; preserve | src/store/tests/code_text.rs | none | infrastructure::store | src/store/code_text.rs | retain |
| src/store/code_row.rs | comemory::store::code_row; preserve | src/store/tests/code_row.rs | none | infrastructure::store | src/store/code_row.rs | retain |
| src/store/code_signals.rs | comemory::store::code_signals; preserve | src/store/tests/code_signals.rs | none | infrastructure::store | src/store/code_signals.rs | retain |
| src/store/code_sync.rs | comemory::store::code_sync; preserve | src/store/tests/code_sync.rs | none | infrastructure::store | src/store/code_sync.rs | retain |
| src/store/connection.rs | comemory::store::connection; preserve | src/store/tests/connection.rs | none | infrastructure::store | src/store/connection.rs | retain |
| src/store/doctor_probes.rs | comemory::store::doctor_probes; preserve | src/store/tests/doctor_probes.rs | none | infrastructure::store | src/store/doctor_probes.rs | retain |
| src/store/document_fts.rs | comemory::store::document_fts; preserve | src/store/tests/document_fts.rs | none | infrastructure::store | src/store/document_fts.rs | retain |
| src/store/documents.rs | comemory::store::documents; preserve | src/store/tests/documents.rs | none | infrastructure::store | src/store/documents.rs | retain |
| src/store/edge_fts.rs | comemory::store::edge_fts; preserve | src/store/tests/edge_fts.rs | none | infrastructure::store | src/store/edge_fts.rs | retain |
| src/store/edges.rs | comemory::store::edges; preserve | src/store/tests/edges.rs | none | infrastructure::store | src/store/edges.rs | retain |
| src/store/edges_neighbors.rs | private | none | none | infrastructure::store | src/store/edges_neighbors.rs | retain |
| src/store/edges_retrieval.rs | comemory::store::edges_retrieval; preserve | src/store/tests/edges_retrieval.rs | none | infrastructure::store | src/store/edges_retrieval.rs | retain |
| src/store/embed.rs | comemory::store::embed; preserve | src/store/tests/embed.rs | none | infrastructure::store | src/store/embed.rs | retain |
| src/store/eval_runs.rs | comemory::store::eval_runs; preserve | src/store/tests/eval_runs.rs | none | infrastructure::store | src/store/eval_runs.rs | retain |
| src/store/feedback.rs | comemory::store::feedback; preserve | src/store/tests/feedback.rs | none | infrastructure::store | src/store/feedback.rs | retain |
| src/store/fts.rs | comemory::store::fts; preserve | src/store/tests/fts.rs; src/store/tests/fts_2.rs | none | infrastructure::store | src/store/fts.rs | retain |
| src/store/fts_memory.rs | comemory::store::fts_memory; preserve | src/store/tests/fts_memory.rs | none | infrastructure::store | src/store/fts_memory.rs | retain |
| src/store/gc_learning.rs | comemory::store::gc_learning; preserve | src/store/tests/gc_learning.rs | none | infrastructure::store | src/store/gc_learning.rs | retain |
| src/store/gc_runs.rs | comemory::store::gc_runs; preserve | src/store/tests/gc_runs.rs | none | infrastructure::store | src/store/gc_runs.rs | retain |
| src/store/index_failures.rs | comemory::store::index_failures; preserve | src/store/tests/index_failures.rs | none | infrastructure::store | src/store/index_failures.rs | retain |
| src/store/index_runs.rs | comemory::store::index_runs; preserve | src/store/tests/index_runs.rs | none | infrastructure::store | src/store/index_runs.rs | retain |
| src/store/indexed_files.rs | comemory::store::indexed_files; preserve | src/store/tests/indexed_files.rs | none | infrastructure::store | src/store/indexed_files.rs | retain |
| src/store/memory_list.rs | comemory::store::memory_list; preserve | src/store/tests/memory_list.rs | none | infrastructure::store | src/store/memory_list.rs | retain |
| src/store/memory_meta.rs | comemory::store::memory_meta; preserve | src/store/tests/memory_meta.rs | none | infrastructure::store | src/store/memory_meta.rs | retain |
| src/store/memory_purge.rs | comemory::store::memory_purge; preserve | src/store/tests/memory_purge.rs | none | infrastructure::store | src/store/memory_purge.rs | retain |
| src/store/memory_row.rs | comemory::store::memory_row; preserve | src/store/tests/memory_row.rs | none | infrastructure::store | src/store/memory_row.rs | retain |
| src/store/memory_signals.rs | private | none | none | infrastructure::store | src/store/memory_signals.rs | retain |
| src/store/migrate.rs | comemory::store::migrate; preserve | src/store/tests/matrix.rs; src/store/tests/migrate.rs; src/store/tests/migrate_2.rs; src/store/tests/migrate_v14.rs; src/store/tests/migrate_v15.rs; src/store/tests/migrate_v4.rs; src/store/tests/migrate_v8.rs | migrations/0001_schema_meta.sql; migrations/0002_v2_tables.sql; migrations/0003_stats_tables.sql; migrations/0004_v4_rank.sql; migrations/0005_v5_learning.sql; migrations/0006_v6_code_graph.sql; migrations/0007_v7_repo_root.sql; migrations/0008_v8_reinforcement.sql; migrations/0009_v9_code_refs.sql; migrations/0010_v10_bandit.sql; migrations/0011_v11_memory_rank.sql; migrations/0012_v12_edge_fts.sql; migrations/0013_v13_documents.sql; migrations/0014_v14_console.sql; migrations/0015_v15_console_api.sql; migrations/0016_v16_sync.sql; migrations/0017_sync_repush.sql; migrations/0018_scheme_path_refs.sql; migrations/0019_query_performance.sql; migrations/0020_candidate_observations.sql | infrastructure::store | src/store/migrate.rs | retain |
| src/store/migrate/backup.rs | private | src/store/migrate/tests/backup.rs | none | infrastructure::store | src/store/migrate/backup.rs | retain |
| src/store/migrate/list.rs | comemory::store::migrate::list; preserve | src/store/migrate/tests/list.rs | none | infrastructure::store | src/store/migrate/list.rs | retain |
| src/store/migrate/preflight.rs | private | src/store/migrate/tests/preflight.rs | none | infrastructure::store | src/store/migrate/preflight.rs | retain |
| src/store/orm.rs | private | src/store/tests/orm.rs | none | infrastructure::store | src/store/orm.rs | retain |
| src/store/prune_apply.rs | comemory::store::prune_apply; preserve | src/store/tests/prune_apply.rs | none | infrastructure::store | src/store/prune_apply.rs | retain |
| src/store/prune_signals.rs | comemory::store::prune_signals; preserve | src/store/tests/prune_signals.rs | none | infrastructure::store | src/store/prune_signals.rs | retain |
| src/store/query_expansions.rs | comemory::store::query_expansions; preserve | src/store/tests/query_expansions.rs | none | infrastructure::store | src/store/query_expansions.rs | retain |
| src/store/random_id.rs | comemory::store::random_id; preserve | src/store/tests/random_id.rs | none | infrastructure::store | src/store/random_id.rs | retain |
| src/store/rebuild_copy.rs | comemory::store::rebuild_copy; preserve | src/store/tests/rebuild_copy.rs | none | infrastructure::store | src/store/rebuild_copy.rs | retain |
| src/store/rebuild_copy_code.rs | comemory::store::rebuild_copy_code; preserve | none | none | infrastructure::store | src/store/rebuild_copy_code.rs | retain |
| src/store/rebuild_copy_documents.rs | comemory::store::rebuild_copy_documents; preserve | src/store/tests/rebuild_copy_documents.rs | none | infrastructure::store | src/store/rebuild_copy_documents.rs | retain |
| src/store/rebuild_copy_history.rs | comemory::store::rebuild_copy_history; preserve | none | none | infrastructure::store | src/store/rebuild_copy_history.rs | retain |
| src/store/rebuild_copy_learning.rs | comemory::store::rebuild_copy_learning; preserve | none | none | infrastructure::store | src/store/rebuild_copy_learning.rs | retain |
| src/store/rebuild_copy_learning_events.rs | comemory::store::rebuild_copy_learning_events; preserve | none | none | infrastructure::store | src/store/rebuild_copy_learning_events.rs | retain |
| src/store/repo_drop.rs | comemory::store::repo_drop; preserve | src/store/tests/repo_drop.rs | none | infrastructure::store | src/store/repo_drop.rs | retain |
| src/store/repo_marker.rs | comemory::store::repo_marker; preserve | src/store/tests/repo_marker.rs | none | infrastructure::store | src/store/repo_marker.rs | retain |
| src/store/repo_marker_roots.rs | comemory::store::repo_marker_roots; preserve | src/store/tests/repo_marker_roots.rs | none | infrastructure::store | src/store/repo_marker_roots.rs | retain |
| src/store/repos_inventory.rs | comemory::store::repos_inventory; preserve | src/store/tests/repos_inventory.rs | none | infrastructure::store | src/store/repos_inventory.rs | retain |
| src/store/retrieval_log.rs | comemory::store::retrieval_log; preserve | src/store/tests/retrieval_log.rs | none | infrastructure::store | src/store/retrieval_log.rs | retain |
| src/store/schema.rs | comemory::store::schema; preserve | src/store/tests/schema.rs; src/store/tests/schema_drift.rs; src/store/tests/schema_fidelity.rs | none | infrastructure::store | src/store/schema.rs | retain |
| src/store/schema_code.rs | comemory::store::schema_code; preserve | none | none | infrastructure::store | src/store/schema_code.rs | retain |
| src/store/schema_core.rs | comemory::store::schema_core; preserve | none | none | infrastructure::store | src/store/schema_core.rs | retain |
| src/store/schema_documents.rs | comemory::store::schema_documents; preserve | none | none | infrastructure::store | src/store/schema_documents.rs | retain |
| src/store/schema_graph.rs | comemory::store::schema_graph; preserve | none | none | infrastructure::store | src/store/schema_graph.rs | retain |
| src/store/schema_history.rs | comemory::store::schema_history; preserve | none | none | infrastructure::store | src/store/schema_history.rs | retain |
| src/store/schema_journal.rs | comemory::store::schema_journal; preserve | src/store/tests/schema_journal.rs | none | infrastructure::store | src/store/schema_journal.rs | retain |
| src/store/schema_learning.rs | comemory::store::schema_learning; preserve | none | none | infrastructure::store | src/store/schema_learning.rs | retain |
| src/store/schema_memory.rs | comemory::store::schema_memory; preserve | none | none | infrastructure::store | src/store/schema_memory.rs | retain |
| src/store/schema_meta.rs | comemory::store::schema_meta; preserve | src/store/tests/schema_meta.rs | none | infrastructure::store | src/store/schema_meta.rs | retain |
| src/store/schema_sync.rs | comemory::store::schema_sync; preserve | none | none | infrastructure::store | src/store/schema_sync.rs | retain |
| src/store/simhash_scan.rs | comemory::store::simhash_scan; preserve | src/store/tests/simhash_scan.rs | none | infrastructure::store | src/store/simhash_scan.rs | retain |
| src/store/sources.rs | comemory::store::sources; preserve | src/store/tests/sources.rs | none | infrastructure::store | src/store/sources.rs | retain |
| src/store/stats_counts.rs | comemory::store::stats_counts; breaking 0.36.0 docs/guides/runtime-orm.md#rust-api | src/store/tests/stats_counts.rs | none | infrastructure::store | src/store/stats_counts.rs | retain |
| src/store/sync_binding.rs | comemory::store::sync_binding; preserve | src/store/tests/sync_binding.rs | none | infrastructure::store | src/store/sync_binding.rs | retain |
| src/store/sync_log.rs | comemory::store::sync_log; preserve | src/store/tests/sync_log.rs | none | infrastructure::store | src/store/sync_log.rs | retain |
| src/store/sync_manifest.rs | comemory::store::sync_manifest; preserve | none | none | infrastructure::store | src/store/sync_manifest.rs | retain |
| src/store/sync_state.rs | comemory::store::sync_state; preserve | src/store/tests/sync_state.rs | none | infrastructure::store | src/store/sync_state.rs | retain |
| src/store/tokenizer.rs | comemory::store::tokenizer; preserve | none | none | infrastructure::store | src/store/tokenizer.rs | retain |
| src/store/tokenizer/ffi.rs | comemory::store::tokenizer::ffi; preserve | src/store/tokenizer/tests/ffi.rs | none | infrastructure::store | src/store/tokenizer/ffi.rs | retain |
| src/store/tokenizer/split.rs | comemory::store::tokenizer::split; preserve | src/store/tokenizer/tests/split.rs | none | infrastructure::store | src/store/tokenizer/split.rs | retain |
| src/store/trash_list.rs | comemory::store::trash_list; preserve | src/store/tests/trash_list.rs | none | infrastructure::store | src/store/trash_list.rs | retain |
| src/store/vector.rs | comemory::store::vector; preserve | src/store/tests/vector.rs | none | infrastructure::store | src/store/vector.rs | retain |
| src/test_common.rs | private | tests/common/auth_fixture.rs; tests/common/benchmark_corpus.rs; tests/common/cli_eval_support.rs; tests/common/cli_prune_support.rs; tests/common/cli_rebuild_support.rs; tests/common/code_rerank_support.rs; tests/common/code_seed.rs; tests/common/code_sync_fixture.rs; tests/common/device_auth_server.rs; tests/common/docs_fixtures.rs; tests/common/document_writer_support.rs; tests/common/git_commit.rs; tests/common/git_repo.rs; tests/common/git_sample.rs; tests/common/git_worktree.rs; tests/common/runner.rs; tests/common/serve_learning_support.rs; tests/common/serve_state.rs; tests/common/sync_platform_server.rs; tests/common/vectors.rs | none | shared::root | src/test_common.rs | retain |
| src/utilities.rs | comemory::utilities; preserve | none | none | shared::utilities | src/utilities.rs | retain |
| src/utilities/blocking.rs | comemory::utilities::blocking; preserve | none | none | shared::utilities | src/utilities/blocking.rs | retain |
| src/utilities/context.rs | comemory::utilities::context; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/utilities/tests/context.rs | none | shared::utilities | src/utilities/context.rs | retain |
| src/utilities/dated_id.rs | private | src/utilities/tests/dated_id.rs | none | shared::utilities | src/utilities/dated_id.rs | retain |
| src/utilities/digest.rs | private | src/utilities/tests/digest.rs | none | shared::utilities | src/utilities/digest.rs | retain |
| src/utilities/embed.rs | comemory::utilities::embed; crate-root-alias | src/utilities/tests/embed.rs | none | shared::utilities | src/utilities/embed.rs | retain |
| src/utilities/embedding_input.rs | private | src/utilities/tests/embedding_input.rs | none | shared::utilities | src/utilities/embedding_input.rs | retain |
| src/utilities/error_code.rs | comemory::utilities::error_code; preserve | src/utilities/tests/error_code.rs | none | shared::utilities | src/utilities/error_code.rs | retain |
| src/utilities/fetch.rs | comemory::utilities::fetch; crate-root-alias | none | none | shared::utilities | src/utilities/fetch.rs | retain |
| src/utilities/file_lock.rs | comemory::utilities::file_lock; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/utilities/tests/file_lock.rs | none | shared::utilities | src/utilities/file_lock.rs | retain |
| src/utilities/http_error.rs | comemory::utilities::http_error; crate-root-alias | none | none | shared::utilities | src/utilities/http_error.rs | retain |
| src/utilities/id_list.rs | private | src/utilities/tests/id_list.rs | none | shared::utilities | src/utilities/id_list.rs | retain |
| src/utilities/pagination.rs | comemory::utilities::pagination; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/utilities/tests/pagination.rs | none | shared::utilities | src/utilities/pagination.rs | retain |
| src/utilities/path_containment.rs | comemory::utilities::path_containment; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/utilities/tests/path_containment.rs | none | shared::utilities | src/utilities/path_containment.rs | retain |
| src/utilities/progress.rs | comemory::utilities::progress; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | shared::utilities | src/utilities/progress.rs | retain |
| src/utilities/query_id.rs | comemory::utilities::query_id; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/utilities/tests/query_id.rs | none | shared::utilities | src/utilities/query_id.rs | retain |
| src/utilities/ref_args.rs | comemory::utilities::ref_args; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | shared::utilities | src/utilities/ref_args.rs | retain |
| src/utilities/repo_root.rs | comemory::utilities::repo_root; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | src/utilities/tests/repo_root.rs | none | shared::utilities | src/utilities/repo_root.rs | retain |
| src/utilities/simhash.rs | comemory::utilities::simhash; crate-root-alias | src/utilities/tests/simhash.rs | none | shared::utilities | src/utilities/simhash.rs | retain |
| src/utilities/telemetry.rs | private | none | none | shared::utilities | src/utilities/telemetry.rs | retain |
| src/utilities/vector_stdin.rs | private | none | none | shared::utilities | src/utilities/vector_stdin.rs | retain |
| src/utilities/when.rs | comemory::utilities::when; breaking 0.34.0 docs/designs/2026-09-17-domain-first-migration-inventory.md#rust-module-path-release-note-for-0340 | none | none | shared::utilities | src/utilities/when.rs | retain |
