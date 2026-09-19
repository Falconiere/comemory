# Integrations

Two independent things live here.

## `agent/` — bundled agent integration

`agent/` is the standalone comemory plugin embedded by `src/domains/integrations/install/bundle.rs`.
The binary builds local Claude Code and Codex marketplace catalogs around it.
The plugin has no toolu runtime dependency; `.toolu/skills/` remains the project
procedure directory for compatibility.

See [installation and migration](../docs/guides/agent-integration.md).

## `reranker/` — optional reference reranker backend

`reranker/` is the optional Python cross-encoder backend for the versioned
reranker command protocol: a pinned Transformers sequence-classification model
with PEFT adapter loading, plus a deterministic non-neural mode that proves
protocol conformance with no model download.

Unlike `agent/`, it is **not** embedded in the binary — `bundle.rs` names every
file it carries and none of them is here — and nothing on comemory's search path
calls it. It is a repository asset an operator opts into.

See [integrations/reranker/README.md](reranker/README.md) and the design,
[docs/designs/2026-09-18-reranker-reference-backend.md](../docs/designs/2026-09-18-reranker-reference-backend.md).

## `training/` — offline LoRA training and qualification

`training/` is the pinned, external recipe that fits a LoRA adapter to a
`comemory export-dataset` output, plus the harness that compares deterministic
ranking, unadapted reranking and LoRA reranking over one captured candidate pool
and records `go`, `no-go` or `insufficient-evidence` against predeclared budgets.

Like `reranker/` and unlike `agent/`, it is **not** embedded in the binary, and
nothing in comemory invokes it. Nothing here runs automatically either: no hook,
watcher, save, sync or index event starts a training job.

See [integrations/training/README.md](training/README.md) and the design,
[docs/designs/2026-09-18-lora-adapter-training.md](../docs/designs/2026-09-18-lora-adapter-training.md).
