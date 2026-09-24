CREATE TABLE IF NOT EXISTS "document_share" (
    "document_id" TEXT NOT NULL REFERENCES "documents"("id") ON DELETE CASCADE,
    "repo" TEXT NOT NULL,
    "shared_id" TEXT NOT NULL,
    "path" TEXT NOT NULL,
    "blocked_reason" TEXT,
    "updated_at" TEXT NOT NULL,
    PRIMARY KEY ("document_id")
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "remote_document" (
    "repo" TEXT NOT NULL,
    "shared_id" TEXT NOT NULL,
    "path" TEXT NOT NULL,
    "title" TEXT NOT NULL,
    "format" TEXT NOT NULL CHECK (format IN ('txt', 'markdown', 'html', 'delimited')),
    "revision_hash" TEXT NOT NULL,
    "chunk_count" INTEGER NOT NULL,
    "accepted_at" TEXT NOT NULL,
    PRIMARY KEY ("repo", "shared_id")
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "remote_document_chunk" (
    "repo" TEXT NOT NULL,
    "shared_id" TEXT NOT NULL,
    "ordinal" INTEGER NOT NULL,
    "heading_path" TEXT NOT NULL DEFAULT (''),
    "char_start" INTEGER NOT NULL,
    "char_end" INTEGER NOT NULL,
    "line_start" INTEGER NOT NULL,
    "line_end" INTEGER NOT NULL,
    "simhash" INTEGER NOT NULL,
    "text" TEXT NOT NULL,
    PRIMARY KEY ("repo", "shared_id", "ordinal")
);

--> statement-breakpoint

CREATE VIRTUAL TABLE IF NOT EXISTS "remote_document_fts" USING "fts5"("repo" UNINDEXED, "shared_id" UNINDEXED, "ordinal" UNINDEXED, "title", "headings", "passage", "path_tokens", tokenize = 'identifier');

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "remote_document_link" (
    "repo" TEXT NOT NULL,
    "shared_id" TEXT NOT NULL,
    "ordinal" INTEGER NOT NULL,
    "target" TEXT NOT NULL,
    PRIMARY KEY ("repo", "shared_id", "ordinal", "target")
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "repository_approval" (
    "label" TEXT PRIMARY KEY,
    "canonical" TEXT NOT NULL,
    "updated_at" TEXT NOT NULL
);

--> statement-breakpoint

CREATE UNIQUE INDEX IF NOT EXISTS "uq_document_share_shared" ON "document_share" ("repo", "shared_id");

--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_remote_document_path" ON "remote_document" ("repo", "path");