CREATE TABLE IF NOT EXISTS "code_generation" (
    "repo" TEXT NOT NULL,
    "generation_id" TEXT NOT NULL,
    "parent_id" TEXT,
    "head" TEXT NOT NULL,
    "mined_commit" TEXT,
    "origin" TEXT NOT NULL CHECK (origin IN ('local', 'sync')),
    "state" TEXT NOT NULL DEFAULT ('staged') CHECK (state IN ('staged', 'active', 'superseded')),
    "file_count" INTEGER NOT NULL,
    "manifest_digest" TEXT NOT NULL,
    "created_at" TEXT NOT NULL,
    "activated_at" TEXT,
    PRIMARY KEY ("repo", "generation_id")
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "remote_code_edge" (
    "repo" TEXT NOT NULL,
    "generation_id" TEXT NOT NULL,
    "rel" TEXT NOT NULL CHECK (rel IN ('imports', 'co_changed')),
    "src_path" TEXT NOT NULL,
    "dst_path" TEXT NOT NULL,
    "weight" INTEGER NOT NULL DEFAULT (1),
    "anchor" TEXT,
    PRIMARY KEY ("repo", "generation_id", "rel", "src_path", "dst_path")
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "remote_code_file" (
    "repo" TEXT NOT NULL,
    "generation_id" TEXT NOT NULL,
    "path" TEXT NOT NULL,
    "blob_oid" TEXT NOT NULL,
    PRIMARY KEY ("repo", "generation_id", "path")
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "remote_code_symbol" (
    "repo" TEXT NOT NULL,
    "generation_id" TEXT NOT NULL,
    "path" TEXT NOT NULL,
    "symbol" TEXT NOT NULL,
    "kind" TEXT NOT NULL,
    "lang" TEXT NOT NULL,
    "line_start" INTEGER NOT NULL,
    "line_end" INTEGER NOT NULL,
    PRIMARY KEY ("repo", "generation_id", "path", "symbol", "line_start")
);

--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_code_generation_state" ON "code_generation" ("repo", "state");

--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_code_generation_origin" ON "code_generation" ("origin", "state");

--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_remote_code_edge_src" ON "remote_code_edge" ("repo", "generation_id", "src_path");

--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_remote_code_symbol_file" ON "remote_code_symbol" ("repo", "generation_id", "path");