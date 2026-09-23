CREATE TABLE IF NOT EXISTS "repository_approval" (
    "label" TEXT PRIMARY KEY,
    "canonical" TEXT NOT NULL,
    "updated_at" TEXT NOT NULL
);