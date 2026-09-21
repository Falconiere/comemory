CREATE TABLE IF NOT EXISTS "activity_log" (
    "id" INTEGER PRIMARY KEY AUTOINCREMENT,
    "at" TEXT NOT NULL,
    "command" TEXT NOT NULL,
    "source" TEXT NOT NULL CHECK (source IN ('cli', 'http', 'mcp')),
    "actor" TEXT,
    "repo" TEXT,
    "duration_ms" INTEGER NOT NULL,
    "ok" INTEGER NOT NULL DEFAULT (1),
    "error_code" TEXT,
    "summary" TEXT
);

--> statement-breakpoint

ALTER TABLE "gc_runs" ADD COLUMN "activity_rows" INTEGER NOT NULL DEFAULT (0);

--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_activity_log_at" ON "activity_log" ("at" DESC, "id");

--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_activity_log_command_at" ON "activity_log" ("command", "at" DESC, "id");