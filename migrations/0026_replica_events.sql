CREATE TABLE IF NOT EXISTS "replica_device" (
    "id" INTEGER PRIMARY KEY CHECK (id = 1),
    "device_id" TEXT NOT NULL,
    "created_at" TEXT NOT NULL
);

--> statement-breakpoint

ALTER TABLE "activity_log" ADD COLUMN "device" TEXT;

--> statement-breakpoint

ALTER TABLE "activity_log" ADD COLUMN "event_id" TEXT;

--> statement-breakpoint

ALTER TABLE "feedback_events" ADD COLUMN "actor" TEXT;

--> statement-breakpoint

ALTER TABLE "feedback_events" ADD COLUMN "device" TEXT;

--> statement-breakpoint

ALTER TABLE "feedback_events" ADD COLUMN "event_id" TEXT;

--> statement-breakpoint

ALTER TABLE "feedback_events" ADD COLUMN "surface" TEXT CHECK (surface IN ('cli', 'http', 'mcp'));

--> statement-breakpoint

ALTER TABLE "replica_payload" ADD COLUMN "redaction" TEXT CHECK (redaction IN ('erased', 'expired'));

--> statement-breakpoint

CREATE UNIQUE INDEX IF NOT EXISTS "uq_activity_log_event_id" ON "activity_log" ("event_id");

--> statement-breakpoint

CREATE UNIQUE INDEX IF NOT EXISTS "uq_feedback_events_event_id" ON "feedback_events" ("event_id");

--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_replica_feed_kind_at" ON "replica_feed" ("entity_kind", "at");

--> statement-breakpoint

-- This database's device id (#254): 16 random bytes from SQLite's randomness
-- source (the OS's, through the unix VFS), minted once — the migration runs
-- once per database, keyed by its schema_meta marker.
INSERT OR IGNORE INTO "replica_device" ("id", "device_id", "created_at")
VALUES (1, lower(hex(randomblob(16))), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
