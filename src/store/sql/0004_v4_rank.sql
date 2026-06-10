-- v4: rank-blend core — access tracking, memory simhash, identifier-tokenized FTS.
--
-- `memories.simhash` is added with DEFAULT 0 as a placeholder; the real
-- values are computed in Rust by `migrate::backfill_memory_simhash`
-- immediately after this migration is applied (SQLite cannot run the
-- SipHash-based SimHash in SQL). `code_symbols.simhash` already exists
-- since 0002.

ALTER TABLE memories ADD COLUMN access_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN last_accessed TEXT;
ALTER TABLE memories ADD COLUMN simhash INTEGER NOT NULL DEFAULT 0;
ALTER TABLE code_symbols ADD COLUMN access_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE code_symbols ADD COLUMN last_accessed TEXT;

UPDATE memories     SET last_accessed = created_at WHERE last_accessed IS NULL;
UPDATE code_symbols SET last_accessed = indexed_at WHERE last_accessed IS NULL;

-- Rebuild FTS tables with the identifier tokenizer (registered per
-- connection in store::connection::open before migrations run). Content
-- re-derives from base tables, so DROP + CREATE + INSERT is safe; the
-- whole migration runs inside one transaction in migrate::apply.
DROP TABLE memory_fts;
CREATE VIRTUAL TABLE memory_fts USING fts5(
    memory_id UNINDEXED,
    body,
    tags,
    tokenize = 'porter identifier'
);
INSERT INTO memory_fts(memory_id, body, tags)
SELECT m.id,
       m.body,
       COALESCE((SELECT group_concat(t.tag, ',')
                   FROM memory_tags t WHERE t.memory_id = m.id), '')
  FROM memories m
 WHERE m.deleted_at IS NULL;

DROP TABLE code_fts;
CREATE VIRTUAL TABLE code_fts USING fts5(
    symbol_id UNINDEXED,
    symbol,
    snippet,
    path_tokens,
    tokenize = 'identifier'
);
-- The identifier tokenizer splits on '/', '.', '-' and camelCase/digit
-- boundaries itself, so the raw path is a valid path_tokens source.
INSERT INTO code_fts(symbol_id, symbol, snippet, path_tokens)
SELECT id, symbol, snippet, path FROM code_symbols;
