-- v11: memories.rank_score — PageRank over the live-memory graph,
-- materialized by `graph::memory_rank` and consumed as the fifth clamped
-- rerank prior.
--
-- Additive defaulted column, so no create-copy-drop-rename is needed: the
-- 0006 `code_symbols.rank_score` ALTER is the precedent (only the `edges`
-- CHECK change in that migration forced a table rebuild). Idempotency comes
-- from the `schema_meta` marker gate in `migrate::apply`, not from the SQL.
--
-- No backfill pass: upgraded rows sit at the neutral 0.0 default until the
-- first save/rebuild/delete trigger recomputes, and a pool whose scores are
-- all 0.0 yields a median of 0.0, which the rank prior reads as "unranked →
-- neutral 1.0". An upgraded database therefore ranks exactly as it did
-- before this column existed.

ALTER TABLE memories ADD COLUMN rank_score REAL NOT NULL DEFAULT 0.0;
