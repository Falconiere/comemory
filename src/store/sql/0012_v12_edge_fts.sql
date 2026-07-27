-- v12: `edge_fts` — the FTS5 triplet index over `edges`, rendering every
-- edge as searchable `src_text / rel_text / dst_text` for `comemory edges`.
--
-- In-migration FTS DDL follows the 0004 precedent: the `identifier`
-- tokenizer is registered per connection in `store::connection::open`
-- *before* migrations run, so it is available to this CREATE.
--
-- The table is created EMPTY and deliberately not backfilled here. Triplet
-- rendering (per-kind CASE over the `memories` joins plus kind-prefix
-- stripping) lives once, in `store::edge_fts::refresh`; restating it as
-- migration SQL would duplicate the rule in two languages and let the two
-- drift. Upgraded databases therefore start empty and self-heal on the
-- first `comemory edges` run, which refreshes whenever `edge_fts` is empty
-- while `edges` is not.
--
-- Payload columns are UNINDEXED so the raw edge rides along for output
-- without polluting the term index (the 0002 non-contentless +
-- UNINDEXED-payload pattern). No FTS triggers: refresh is an explicit
-- DELETE + INSERT, as everywhere else in this schema.

CREATE VIRTUAL TABLE edge_fts USING fts5(
    src_text,
    rel_text,
    dst_text,
    src_kind UNINDEXED,
    src_id UNINDEXED,
    rel UNINDEXED,
    dst_kind UNINDEXED,
    dst_id UNINDEXED,
    weight UNINDEXED,
    tokenize = 'identifier'
);
