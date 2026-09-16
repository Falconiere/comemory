-- Indexed graph citations, recent retrievals, and stable paginated listings.
CREATE INDEX idx_edges_citation ON edges(rel, dst_id, src_id)
    WHERE src_kind = 'memory';
CREATE INDEX idx_retrieval_log_recent ON retrieval_log(at DESC, query_id DESC);
CREATE INDEX idx_retrieval_log_source_at ON retrieval_log(source, at);
DROP INDEX idx_index_runs_started;
CREATE INDEX idx_index_runs_started ON index_runs(started_at DESC, id);
CREATE INDEX idx_index_runs_repo_started ON index_runs(repo, started_at DESC, id);
CREATE INDEX idx_memories_created ON memories(created_at DESC, id)
    WHERE deleted_at IS NULL;
CREATE INDEX idx_memories_repo_created ON memories(repo, created_at DESC, id)
    WHERE deleted_at IS NULL;
CREATE INDEX idx_memories_kind_created ON memories(kind, created_at DESC, id)
    WHERE deleted_at IS NULL;
CREATE INDEX idx_memories_repo_kind_created ON memories(repo, kind, created_at DESC, id)
    WHERE deleted_at IS NULL;

-- Candidate index only: the listing still checks literal LIKE semantics.
-- External content keeps bodies in memories, without another stored copy.
CREATE VIRTUAL TABLE memory_substring USING fts5(
    body,
    tokenize = 'trigram',
    content = 'memories',
    content_rowid = 'rowid',
    columnsize = 0
);
CREATE TRIGGER memories_substring_insert AFTER INSERT ON memories BEGIN
    INSERT INTO memory_substring(rowid, body) VALUES(new.rowid, new.body);
END;
CREATE TRIGGER memories_substring_delete AFTER DELETE ON memories BEGIN
    INSERT INTO memory_substring(memory_substring, rowid, body)
        VALUES('delete', old.rowid, old.body);
END;
CREATE TRIGGER memories_substring_update AFTER UPDATE ON memories
WHEN old.body IS NOT new.body OR old.rowid != new.rowid BEGIN
    INSERT INTO memory_substring(memory_substring, rowid, body)
        VALUES('delete', old.rowid, old.body);
    INSERT INTO memory_substring(rowid, body) VALUES(new.rowid, new.body);
END;
INSERT INTO memory_substring(memory_substring) VALUES('rebuild');
