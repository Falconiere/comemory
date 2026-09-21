CREATE TABLE IF NOT EXISTS "replica_cursor" (
    "workspace_id" TEXT PRIMARY KEY,
    "api_url" TEXT NOT NULL,
    "stream_epoch" TEXT NOT NULL,
    "applied_sequence" INTEGER NOT NULL DEFAULT (0),
    "updated_at" TEXT NOT NULL
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "replica_feed" (
    "sequence" INTEGER PRIMARY KEY AUTOINCREMENT,
    "epoch" TEXT NOT NULL,
    "entity_kind" TEXT NOT NULL,
    "entity_key" TEXT NOT NULL,
    "op" TEXT NOT NULL CHECK (op IN ('upsert', 'tombstone', 'restore')),
    "payload_digest" TEXT,
    "schema_version" INTEGER NOT NULL,
    "operation_id" TEXT NOT NULL,
    "origin" TEXT NOT NULL CHECK (origin IN ('local', 'sync')),
    "repository" TEXT,
    "at" TEXT NOT NULL
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "replica_operation" (
    "operation_id" TEXT PRIMARY KEY,
    "entity_kind" TEXT NOT NULL,
    "entity_key" TEXT NOT NULL,
    "op" TEXT NOT NULL CHECK (op IN ('upsert', 'tombstone', 'restore')),
    "payload_digest" TEXT,
    "schema_version" INTEGER NOT NULL,
    "repository" TEXT,
    "observed_sequence" INTEGER,
    "state" TEXT NOT NULL DEFAULT ('pending') CHECK (state IN ('pending', 'accepted', 'rejected')),
    "upstream_sequence" INTEGER,
    "disposition" TEXT,
    "attempts" INTEGER NOT NULL DEFAULT (0),
    "last_error" TEXT,
    "created_at" TEXT NOT NULL,
    "updated_at" TEXT NOT NULL
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "replica_payload" (
    "digest" TEXT PRIMARY KEY,
    "entity_kind" TEXT NOT NULL,
    "schema_version" INTEGER NOT NULL,
    "bytes" TEXT,
    "byte_len" INTEGER NOT NULL,
    "created_at" TEXT NOT NULL,
    "redacted_at" TEXT
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "replica_receipt" (
    "operation_id" TEXT PRIMARY KEY,
    "epoch" TEXT NOT NULL,
    "sequence" INTEGER,
    "disposition" TEXT NOT NULL,
    "payload_digest" TEXT,
    "reason" TEXT,
    "accepted_at" TEXT NOT NULL
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "replica_revision" (
    "entity_kind" TEXT NOT NULL,
    "entity_key" TEXT NOT NULL,
    "sequence" INTEGER NOT NULL,
    "payload_digest" TEXT,
    "deleted" INTEGER NOT NULL DEFAULT (0),
    "deleted_sequence" INTEGER,
    "updated_at" TEXT NOT NULL,
    PRIMARY KEY ("entity_kind", "entity_key")
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "replica_staged_part" (
    "staging_id" TEXT NOT NULL,
    "part_index" INTEGER NOT NULL,
    "part_count" INTEGER NOT NULL,
    "bytes" TEXT NOT NULL,
    "created_at" TEXT NOT NULL,
    PRIMARY KEY ("staging_id", "part_index")
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "replica_stream" (
    "id" INTEGER PRIMARY KEY CHECK (id = 1),
    "epoch" TEXT NOT NULL,
    "created_at" TEXT NOT NULL
);

--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_replica_feed_entity" ON "replica_feed" ("entity_kind", "entity_key");

--> statement-breakpoint

CREATE UNIQUE INDEX IF NOT EXISTS "uq_replica_feed_operation" ON "replica_feed" ("operation_id");

--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_replica_operation_state" ON "replica_operation" ("state", "created_at");

--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_replica_staged_part_created" ON "replica_staged_part" ("staging_id", "created_at");