# Qwick Knowledge Platform (RAG) – Decision Brief

## 1. Why We Are Looking at This
Qwick’s policies, playbooks, and operational knowledge live across many tools (docs, ticketing, CRM, internal notes). This creates slow onboarding, inconsistent answers to Pros and Business Partners, and extra work for support, ops, sales, and product teams.

We are exploring a **Retrieval-Augmented Generation (RAG)**–based “central knowledge assistant” that can answer internal questions and draft responses using our existing documentation, while respecting compliance constraints.

## 2. What the System Would Do
- **For Support & Ops:** Suggest answers to policy questions (pay, penalties, cancellations, eligibility) inside the ticketing tool, with links to the underlying policy.
- **For Sales & AM:** Summarize calls/tickets into the CRM and surface relevant policies and case studies.
- **For Product & Leadership:** Make incidents, decisions, and design docs searchable and summarize them quickly.

The system is built on **hybrid search + RAG** and always shows **citations** to source documents, so humans can verify before sending.

## 3. Benefits vs. Risks
**Benefits**
- Productivity: Less time searching and writing; faster onboarding.
- Quality: More consistent policy application; fewer “one-off” interpretations.
- Insight: Analytics on what employees ask, revealing training and product gaps.

**Risks**
- Compliance/privacy if we ingest the wrong data or pick the wrong vendors.
- Hallucinations if the model overstates what policies say.
- Cost if we scale too fast without clear value metrics.

We mitigate these by starting small, enforcing strict access control and redaction, forcing grounded answers with citations, and tracking usage/cost closely.

## 4. Recommended Decision: Small, Measurable Pilot
- Run a **3–4 month MVP** focused on one area: support policy questions for a defined region or queue.
- Use only vetted policy sources (no raw PII), enforce ACLs, and require human review before responses are sent externally.
- Measure: handle time, answer quality, agent satisfaction, and platform cost.

If the pilot does **not** show clear value or manageable risk, we stop or narrow scope. If it does, we can expand gradually to other teams and workflows.
