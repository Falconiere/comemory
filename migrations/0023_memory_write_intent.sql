CREATE TABLE IF NOT EXISTS "memory_needs_embedding" (
    "memory_id" TEXT PRIMARY KEY,
    "reason" TEXT NOT NULL CHECK (reason IN ('absent', 'model', 'dims')),
    "model" TEXT,
    "dims" INTEGER,
    "recorded_at" TEXT NOT NULL
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "memory_write_intent" (
    "entity_key" TEXT PRIMARY KEY,
    "kind" TEXT NOT NULL CHECK (kind IN ('write', 'delete')),
    "md_path" TEXT NOT NULL,
    "operation_id" TEXT NOT NULL,
    "started_at" TEXT NOT NULL
);

--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_memory_needs_embedding_reason" ON "memory_needs_embedding" ("reason", "recorded_at");