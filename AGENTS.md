# Repository Guidelines

## Project Structure & Module Organization
- All content currently lives under `docs/`. Key documents are:
  - `docs/central-knowledge-rag.md` – generic architecture.
  - `docs/qwick-rag-business-case.md` – business value for Qwick.
  - `docs/qwick-rag-risk-compliance-cost.md` – risk and alternatives analysis.
  - `docs/qwick-rag-exec-brief.md` – leadership decision brief.
  - `docs/qwick-rag-mvp-spec.md` – MVP spec and architecture add-ons.
- Add new documents under `docs/` grouped by concern, for example `docs/ingestion/`, `docs/retrieval/`, `docs/orchestration/`, and `docs/interfaces/`.
- When introducing code or configs, mirror the architecture described in the docs (ingestion, indexing, query engine, orchestration, interfaces, observability) and align with the Qwick MVP and risk guidance.

## Build, Test, and Development Commands
- This repository is currently documentation-only; open Markdown files directly in your editor or viewer, e.g. `code docs/central-knowledge-rag.md`.
- If you add tooling, expose common operations via a `Makefile` or `package.json`:
  - `make lint` / `npm run lint` – validate Markdown, links, and configuration.
  - `make test` / `npm test` – run automated tests.
  - `make dev` / `npm run dev` – run any local demo or playground.
- Update this section whenever new commands or tools are introduced.

## Coding Style & Naming Conventions
- Prefer clear, architectural prose over marketing language; keep paragraphs short and focused.
- Use Markdown headings to reflect RAG pipeline stages; avoid deeply nested heading levels when possible.
- Name files by topic and layer, e.g. `ingestion-connectors.md`, `retrieval-strategies.md`, `observability-metrics.md`.
- Keep lines under ~100 characters and use fenced code blocks for examples and configuration snippets.

## Testing Guidelines
- For future executable code, place tests in a top-level `tests/` directory and mirror the source layout.
- Name test files and cases after the behavior, e.g. `test_ingestion_pipeline_handles_retries`.
- When adding complex examples or reference implementations, include minimal executable samples and keep them synchronized with the written design.

## Commit & Pull Request Guidelines
- Use focused commits with imperative subjects, e.g. `Add ingestion metadata guidelines` or `Refine retrieval strategy section`.
- For pull requests, include:
  - A short summary of what changed and why.
  - The primary architecture areas touched (ingestion, retrieval, orchestration, interfaces, observability).
  - Any follow-up tasks or open questions for reviewers.
- Prefer smaller, incremental PRs that evolve the design rather than large restructures without context.

## Security & Configuration Tips
- Do not commit API keys, credentials, or proprietary customer data; use realistic placeholders in examples.
- When documenting configuration, clearly separate secrets from non-sensitive settings and indicate expected storage (e.g. environment variables or a secret manager).
