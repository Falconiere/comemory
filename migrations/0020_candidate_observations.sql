CREATE TABLE IF NOT EXISTS "candidate_judgments" (
    "observation_id" TEXT NOT NULL,
    "candidate_ref" TEXT NOT NULL,
    "domain" TEXT NOT NULL CHECK (domain IN ('memory','code','document')),
    "relevance" INTEGER NOT NULL CHECK (relevance >= 0 AND relevance <= 3),
    "provenance" TEXT NOT NULL CHECK (provenance IN ('manual','implicit')),
    "at" TEXT NOT NULL,
    PRIMARY KEY ("observation_id", "candidate_ref")
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "candidate_observations" (
    "observation_id" TEXT NOT NULL,
    "pool_position" INTEGER NOT NULL,
    "domain" TEXT NOT NULL CHECK (domain IN ('memory','code','document')),
    "candidate_ref" TEXT NOT NULL,
    "content_version" TEXT NOT NULL,
    "unresolved" INTEGER NOT NULL DEFAULT (0),
    "returned_position" INTEGER,
    "retrieval_score" REAL NOT NULL,
    "rank_in_domain" INTEGER NOT NULL,
    "tier" INTEGER,
    "text" TEXT NOT NULL,
    "text_sha256" TEXT NOT NULL,
    "text_full_bytes" INTEGER NOT NULL,
    "text_truncated" INTEGER NOT NULL,
    "locator_json" TEXT NOT NULL,
    PRIMARY KEY ("observation_id", "pool_position")
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "candidate_query_observations" (
    "observation_id" TEXT PRIMARY KEY,
    "observation_version" INTEGER NOT NULL,
    "query_id" TEXT,
    "query" TEXT NOT NULL,
    "source" TEXT NOT NULL,
    "filters_json" TEXT NOT NULL,
    "retrieval_json" TEXT NOT NULL,
    "knobs_hash" TEXT NOT NULL,
    "corpus_digest" TEXT NOT NULL,
    "decay_frozen" INTEGER NOT NULL,
    "pool_size" INTEGER NOT NULL,
    "page_limit" INTEGER NOT NULL,
    "page_offset" INTEGER NOT NULL,
    "candidate_count" INTEGER NOT NULL,
    "truncated" INTEGER NOT NULL,
    "at" TEXT NOT NULL
);

--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_candidate_judgments_ref" ON "candidate_judgments" ("candidate_ref");

--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_candidate_observations_ref" ON "candidate_observations" ("candidate_ref");

--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_candidate_query_observations_at" ON "candidate_query_observations" ("at");