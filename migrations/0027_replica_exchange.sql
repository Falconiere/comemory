CREATE TABLE IF NOT EXISTS "replica_binding" (
    "api_url" TEXT NOT NULL,
    "workspace_id" TEXT NOT NULL,
    "entity_kind" TEXT NOT NULL,
    "entity_key" TEXT NOT NULL,
    "synced_digest" TEXT,
    "synced_deleted" INTEGER NOT NULL DEFAULT (0),
    "synced_sequence" INTEGER,
    "synced_epoch" TEXT,
    "updated_at" TEXT NOT NULL,
    PRIMARY KEY ("api_url", "workspace_id", "entity_kind", "entity_key")
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "replica_pull_hold" (
    "api_url" TEXT NOT NULL,
    "workspace_id" TEXT NOT NULL,
    "stream_epoch" TEXT NOT NULL,
    "from_sequence" INTEGER NOT NULL,
    "to_sequence" INTEGER NOT NULL,
    "reason" TEXT NOT NULL CHECK (reason IN ('policy', 'pending_local', 'server_withheld', 'secret', 'id_collision')),
    "entity_kind" TEXT,
    "entity_key" TEXT,
    "repository" TEXT,
    "policy_revision" INTEGER,
    "recorded_at" TEXT NOT NULL,
    PRIMARY KEY ("api_url", "workspace_id", "stream_epoch", "from_sequence")
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "replica_replay" (
    "api_url" TEXT NOT NULL,
    "workspace_id" TEXT NOT NULL,
    "entity_kind" TEXT NOT NULL,
    "entity_key" TEXT NOT NULL,
    "sequence" INTEGER NOT NULL,
    "entry_json" TEXT NOT NULL,
    PRIMARY KEY ("api_url", "workspace_id", "entity_kind", "entity_key")
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "sync_exchange" (
    "api_url" TEXT NOT NULL,
    "workspace_id" TEXT NOT NULL,
    "protocol" TEXT CHECK (protocol IN ('legacy', 'replica-v1')),
    "coverage_reason" TEXT,
    "selected_at" TEXT,
    "upgrade_through" INTEGER,
    "replay_kind" TEXT,
    "replay_state" TEXT CHECK (replay_state IN ('scanning', 'applying')),
    "replay_scan_through" INTEGER,
    "replay_target" INTEGER,
    "network_state" TEXT NOT NULL DEFAULT ('ok') CHECK (network_state IN ('ok', 'backoff', 'auth_suspended', 'protocol_error')),
    "retry_at" TEXT,
    "consecutive_failures" INTEGER NOT NULL DEFAULT (0),
    "last_error" TEXT,
    "suspended_fingerprint" TEXT,
    "upstream_head" INTEGER,
    "stall_sequence" INTEGER,
    "stall_reason" TEXT,
    "last_session_at" TEXT,
    "last_ok_at" TEXT,
    "updated_at" TEXT NOT NULL,
    PRIMARY KEY ("api_url", "workspace_id")
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "sync_policy_snapshot" (
    "api_url" TEXT NOT NULL,
    "workspace_id" TEXT NOT NULL,
    "revision" INTEGER NOT NULL,
    "fingerprint" TEXT NOT NULL,
    "allowlist_json" TEXT NOT NULL,
    "mappings_json" TEXT NOT NULL,
    "loaded_at" TEXT NOT NULL,
    PRIMARY KEY ("api_url", "workspace_id")
);

--> statement-breakpoint

-- Rebuild "replica_cursor": SQLite cannot alter these columns in place.
-- The runner hoists this pragma outside the transaction, where it is
-- not a no-op, and restores the original setting afterwards.
PRAGMA foreign_keys = OFF;
--> statement-breakpoint
CREATE TABLE "_toolu_new_replica_cursor" (
    "api_url" TEXT NOT NULL,
    "workspace_id" TEXT NOT NULL,
    "stream_epoch" TEXT NOT NULL,
    "applied_sequence" INTEGER NOT NULL DEFAULT (0),
    "anchor_sequence" INTEGER,
    "anchor_operation_id" TEXT,
    "updated_at" TEXT NOT NULL,
    PRIMARY KEY ("api_url", "workspace_id")
);
--> statement-breakpoint
INSERT INTO "_toolu_new_replica_cursor" ("api_url", "workspace_id", "stream_epoch", "applied_sequence", "updated_at") SELECT "api_url", "workspace_id", "stream_epoch", "applied_sequence", "updated_at" FROM "replica_cursor";
--> statement-breakpoint
DROP TABLE "replica_cursor";
--> statement-breakpoint
PRAGMA legacy_alter_table = ON;
--> statement-breakpoint
ALTER TABLE "_toolu_new_replica_cursor" RENAME TO "replica_cursor";
--> statement-breakpoint
PRAGMA legacy_alter_table = OFF;

--> statement-breakpoint

ALTER TABLE "replica_operation" ADD COLUMN "api_url" TEXT;

--> statement-breakpoint

ALTER TABLE "replica_operation" ADD COLUMN "hold_detail" TEXT;

--> statement-breakpoint

ALTER TABLE "replica_operation" ADD COLUMN "hold_reason" TEXT;

--> statement-breakpoint

ALTER TABLE "replica_operation" ADD COLUMN "upstream_epoch" TEXT;

--> statement-breakpoint

ALTER TABLE "replica_operation" ADD COLUMN "wire_repository" TEXT;

--> statement-breakpoint

ALTER TABLE "replica_operation" ADD COLUMN "workspace_id" TEXT;

--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_replica_binding_entity" ON "replica_binding" ("entity_kind", "entity_key");

--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_replica_pull_hold_reason" ON "replica_pull_hold" ("api_url", "workspace_id", "reason");