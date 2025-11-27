# Qwick RAG MVP – Product & Engineering Spec

## 1. Goal and Non-Goals
**Goal (MVP)**  
Assist support agents with **policy-related questions** by suggesting grounded answers inside the ticketing tool, using Qwick’s official policies and playbooks, with citations.

**Non-goals (MVP)**
- No fully autonomous responses to Pros/Business Partners.
- No direct access to raw PII fields or free-text ticket bodies.
- No company-wide knowledge coverage; we focus on a limited region/queue and set of policies.

## 2. Target Users and Workflow
- Primary users: support agents and their leads in a selected queue (e.g., policy/eligibility or cancellations for a specific region).
- Typical workflow:
  1. Agent opens a ticket in the existing tool.
  2. Clicks “Suggest answer” (or uses a shortcut).
  3. System reads the ticket subject + selected fields, retrieves relevant policies, and returns a drafted internal answer with citations.
  4. Agent reviews/edits and sends externally.

## 3. Scope and Data Sources
- **In-scope sources**
  - Official policy docs (pay, penalties, cancellations, eligibility) in existing repositories.
  - Selected internal runbooks and macros approved by support leadership.
- **Out-of-scope (MVP)**
  - Free-text historical tickets, raw chat logs, and CRM notes.
  - Legal documents containing sensitive information not already summarized in policies.

Each document is tagged by owner, last updated, and access level; only documents visible to the requesting agent may be retrieved.

## 4. Functional Requirements
- API that accepts a ticket context and returns:
  - Suggested answer text.
  - List of cited documents with titles and deep links.
- “Grounded only” behavior: model must answer from retrieved content or explicitly say it cannot answer.
- Policy categories configurable (e.g., enable/disable pay vs. cancellations).
- Admin UI or config file to manage which sources are included.

## 5. Non-Functional, Compliance, and Safety
- Data minimization: ticketing integration passes only necessary fields (category, short description, relevant structured attributes).
- No storage of raw PII in the vector store; documents must be pre-sanitized where needed.
- Access control: retrieval respects the same permissions as the ticketing/doc systems.
- Observability: logs capture which sources were used and which suggestions were accepted, without storing full PII or full answers where avoidable.
- Latency: target p95 under 3–5 seconds for a suggestion.

## 6. Technical Approach (High-Level)
- Ingestion jobs to pull and index approved policy documents into a vector store with metadata (category, owner, region, access level).
- RAG service that:
  - Classifies the question type.
  - Retrieves top N relevant snippets, applies filters (region, category, ACLs).
  - Calls an LLM to generate an answer with citations.
- Integration layer for the ticketing system (side panel, macro, or API-based plugin).
- Use smaller or open-weight models where possible; reserve large models for complex queries.

## 7. Success Metrics and Rollout
- Key metrics:
  - Reduction in average handle time for in-scope tickets.
  - Agent-rated answer quality (e.g., 1–5 rating after using a suggestion).
  - Percentage of suggestions used vs. discarded.
  - Platform cost per assisted ticket.
- Rollout:
  - Phase 0: Sandbox and internal dogfooding with a few power users.
  - Phase 1: Limited rollout to one queue/region.
  - Phase 2: Expand policies and regions if metrics and risk profile are acceptable.

## 8. Feature Knowledge & Product Lifecycle Alignment
- Every new or changed feature that affects support/policy behavior must ship with an up-to-date **feature knowledge doc** so the RAG system can be trusted.
- In each relevant product repo (e.g., mobile, web, internal tools), add or update `docs/features/<feature-name>.md` with:
  - Summary (who it’s for, where it lives).
  - Expected behavior (happy path).
  - Edge cases and errors.
  - Policy/finance impact and links to central policy docs.
  - FAQ for Support/Ops, rollout/version, and owner team.
- Definition of done for such features includes: code, tests, and the feature doc updated to reflect actual shipped behavior.
- The ingestion pipeline pulls only curated feature docs from release/main branches, with metadata (e.g., component, user type, feature name, environment, owner team) so RAG answers stay aligned with what is live in production.

## 9. Architecture Add-Ons for Fast & Accurate RAG
- **Hybrid retrieval:** combine vector search with keyword/term search and strong metadata filters (team, region, policy type, feature) to improve recall and precision.
- **Reranking:** use a lightweight reranker on the top retrieved snippets before calling the LLM so answers are grounded in the best evidence.
- **Model routing:** rely on small/cheap models for intent classification and routing, and reserve larger models for complex answer generation.
- **Caching & performance:** cache frequent questions and responses by normalized query + role/region; pre-warm connections to vector/search backends to keep p95 latency under target.
- **Observability & evaluation:** instrument the pipeline (traces, metrics, sampling) and maintain a Qwick policy QA set to regularly evaluate correctness, citations, and refusal behavior.
