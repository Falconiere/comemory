# Qwick Central Knowledge Platform (RAG)

This repository contains the **design and decision docs** for a Retrieval-Augmented Generation (RAG)–based knowledge platform to centralize Qwick’s operational knowledge (policies, playbooks, product docs, support macros, legal/compliance guidance).

The goal is to make it easy for support, ops, sales, and product teams to **find accurate answers quickly**, and for engineering to build a **safe, efficient MVP** with clear guardrails around compliance, hallucinations, and cost.

## Repository Layout
- `AGENTS.md` – contributor guidelines for this repository.
- `docs/central-knowledge-rag.md` – generic central-knowledge RAG architecture and concepts.
- `docs/qwick-rag-business-case.md` – business value, automation opportunities, and cost framing for Qwick.
- `docs/qwick-rag-risk-compliance-cost.md` – analysis of compliance, hallucination, and cost risks plus alternatives.
- `docs/qwick-rag-exec-brief.md` – 1–2 page decision brief for leadership.
- `docs/qwick-rag-mvp-spec.md` – product & engineering spec for a Qwick support-policy RAG MVP, including performance and lifecycle alignment.

## How to Use This Repo
- **Leadership / stakeholders:** start with `docs/qwick-rag-exec-brief.md` and `docs/qwick-rag-business-case.md` to decide whether to run the MVP.
- **Product & engineering:** use `docs/qwick-rag-mvp-spec.md` and `docs/central-knowledge-rag.md` as the basis for technical discovery, estimation, and implementation.
- **Risk, legal, and security:** review `docs/qwick-rag-risk-compliance-cost.md` to assess guardrails and decide acceptable scope.

## Contributing
- Follow `AGENTS.md` for structure and style when adding or changing docs.
- Keep documents aligned: update the MVP spec and risk docs when you introduce new architecture components or change scope.
- When the first implementation starts, add links from this repo to the relevant GitHub code repositories (mobile, backend, web, internal tools) and ensure feature knowledge docs in those repos follow the patterns described here.

