CREATE TABLE IF NOT EXISTS "project_transfer_bindings" (
    "project_id" TEXT PRIMARY KEY REFERENCES "projects"("id") ON DELETE CASCADE,
    "direction" TEXT NOT NULL,
    "remote" TEXT NOT NULL,
    "digest" TEXT NOT NULL,
    "remapped_from_principal_type" TEXT,
    "remapped_from_principal_id" TEXT,
    "transferred_at" INTEGER NOT NULL DEFAULT (unixepoch() * 1000)
);