CREATE TABLE IF NOT EXISTS "project_changes" (
    "seq" INTEGER PRIMARY KEY AUTOINCREMENT,
    "project_id" TEXT NOT NULL,
    "event_id" TEXT NOT NULL,
    "entity_type" TEXT NOT NULL,
    "op" TEXT NOT NULL,
    "created_at" INTEGER NOT NULL
);