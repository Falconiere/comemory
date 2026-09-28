CREATE TABLE IF NOT EXISTS "project_activity_events" (
    "id" TEXT PRIMARY KEY,
    "project_id" TEXT NOT NULL REFERENCES "projects"("id") ON DELETE CASCADE,
    "actor_principal_type" TEXT NOT NULL,
    "actor_principal_id" TEXT NOT NULL,
    "event_type" TEXT NOT NULL,
    "entity_type" TEXT NOT NULL,
    "entity_id" TEXT NOT NULL,
    "payload" TEXT NOT NULL DEFAULT ('{}'),
    "created_at" INTEGER NOT NULL DEFAULT (unixepoch() * 1000)
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "project_approvals" (
    "id" TEXT PRIMARY KEY,
    "project_id" TEXT NOT NULL REFERENCES "projects"("id") ON DELETE CASCADE,
    "proposal_id" TEXT NOT NULL REFERENCES "project_plan_proposals"("id") ON DELETE CASCADE,
    "decision" TEXT NOT NULL,
    "reviewer_principal_type" TEXT NOT NULL,
    "reviewer_principal_id" TEXT NOT NULL,
    "rationale" TEXT,
    "created_at" INTEGER NOT NULL DEFAULT (unixepoch() * 1000)
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "project_command_receipts" (
    "id" TEXT PRIMARY KEY,
    "project_id" TEXT NOT NULL REFERENCES "projects"("id") ON DELETE CASCADE,
    "principal_type" TEXT NOT NULL,
    "principal_id" TEXT NOT NULL,
    "idempotency_key" TEXT NOT NULL,
    "command_type" TEXT NOT NULL,
    "request_digest" TEXT NOT NULL,
    "response" TEXT NOT NULL DEFAULT ('{}'),
    "created_at" INTEGER NOT NULL DEFAULT (unixepoch() * 1000)
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "project_criteria" (
    "id" TEXT PRIMARY KEY,
    "project_id" TEXT NOT NULL REFERENCES "projects"("id") ON DELETE CASCADE,
    "work_item_id" TEXT,
    "description" TEXT NOT NULL,
    "required" INTEGER NOT NULL DEFAULT (1),
    "evidence_requirement" TEXT NOT NULL DEFAULT ('reported'),
    "resolution" TEXT NOT NULL DEFAULT ('open'),
    "resolution_rationale" TEXT,
    "resolver_principal_type" TEXT,
    "resolver_principal_id" TEXT,
    "position" INTEGER NOT NULL DEFAULT (0),
    "archived_at" INTEGER,
    FOREIGN KEY ("work_item_id", "project_id") REFERENCES "project_work_items" ("id", "project_id") ON DELETE CASCADE
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "project_evidence" (
    "id" TEXT PRIMARY KEY,
    "project_id" TEXT NOT NULL REFERENCES "projects"("id") ON DELETE CASCADE,
    "work_item_id" TEXT,
    "execution_id" TEXT,
    "kind" TEXT NOT NULL,
    "source" TEXT NOT NULL,
    "external_id" TEXT,
    "url" TEXT,
    "trust" TEXT NOT NULL DEFAULT ('pending'),
    "metadata" TEXT NOT NULL DEFAULT ('{}'),
    "content_hash" TEXT,
    "verified_by" TEXT,
    "verified_at" INTEGER,
    "creator_principal_type" TEXT NOT NULL,
    "creator_principal_id" TEXT NOT NULL,
    "created_at" INTEGER NOT NULL DEFAULT (unixepoch() * 1000),
    FOREIGN KEY ("work_item_id", "project_id") REFERENCES "project_work_items" ("id", "project_id") ON DELETE CASCADE,
    FOREIGN KEY ("execution_id", "project_id") REFERENCES "project_executions" ("id", "project_id") ON DELETE CASCADE
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "project_evidence_criteria" (
    "evidence_id" TEXT NOT NULL REFERENCES "project_evidence"("id") ON DELETE CASCADE,
    "criterion_id" TEXT NOT NULL REFERENCES "project_criteria"("id") ON DELETE CASCADE,
    PRIMARY KEY ("evidence_id", "criterion_id")
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "project_executions" (
    "id" TEXT PRIMARY KEY,
    "project_id" TEXT NOT NULL REFERENCES "projects"("id") ON DELETE CASCADE,
    "work_item_id" TEXT NOT NULL,
    "actor_principal_type" TEXT NOT NULL,
    "actor_principal_id" TEXT NOT NULL,
    "state" TEXT NOT NULL DEFAULT ('active'),
    "started_at" INTEGER NOT NULL DEFAULT (unixepoch() * 1000),
    "heartbeat_at" INTEGER,
    "finished_at" INTEGER,
    "result_summary" TEXT,
    "version" INTEGER NOT NULL DEFAULT (1),
    "superseded_by_execution_id" TEXT REFERENCES "project_executions"("id"),
    FOREIGN KEY ("work_item_id", "project_id") REFERENCES "project_work_items" ("id", "project_id") ON DELETE CASCADE
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "project_milestones" (
    "id" TEXT PRIMARY KEY,
    "project_id" TEXT NOT NULL REFERENCES "projects"("id") ON DELETE CASCADE,
    "name" TEXT NOT NULL,
    "description" TEXT NOT NULL,
    "target_date" INTEGER NOT NULL,
    "position" INTEGER NOT NULL DEFAULT (0),
    "status" TEXT NOT NULL DEFAULT ('planned'),
    "archived_at" INTEGER
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "project_plan_proposals" (
    "id" TEXT PRIMARY KEY,
    "project_id" TEXT NOT NULL REFERENCES "projects"("id") ON DELETE CASCADE,
    "base_plan_version" INTEGER NOT NULL,
    "state" TEXT NOT NULL DEFAULT ('pending'),
    "operations" TEXT NOT NULL,
    "assumptions" TEXT NOT NULL DEFAULT ('[]'),
    "risks" TEXT NOT NULL DEFAULT ('[]'),
    "rationale" TEXT NOT NULL,
    "proposer_principal_type" TEXT NOT NULL,
    "proposer_principal_id" TEXT NOT NULL,
    "request_digest" TEXT NOT NULL,
    "created_at" INTEGER NOT NULL DEFAULT (unixepoch() * 1000),
    "updated_at" INTEGER NOT NULL DEFAULT (unixepoch() * 1000)
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "project_repositories" (
    "project_id" TEXT NOT NULL REFERENCES "projects"("id") ON DELETE CASCADE,
    "repo" TEXT NOT NULL,
    "creator_principal_type" TEXT NOT NULL,
    "creator_principal_id" TEXT NOT NULL,
    "created_at" INTEGER NOT NULL DEFAULT (unixepoch() * 1000),
    PRIMARY KEY ("project_id", "repo")
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "project_work_item_dependencies" (
    "project_id" TEXT NOT NULL REFERENCES "projects"("id") ON DELETE CASCADE,
    "blocker_id" TEXT NOT NULL,
    "blocked_id" TEXT NOT NULL CHECK (blocker_id != blocked_id),
    PRIMARY KEY ("project_id", "blocker_id", "blocked_id"),
    FOREIGN KEY ("blocker_id", "project_id") REFERENCES "project_work_items" ("id", "project_id") ON DELETE CASCADE,
    FOREIGN KEY ("blocked_id", "project_id") REFERENCES "project_work_items" ("id", "project_id") ON DELETE CASCADE
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "project_work_items" (
    "id" TEXT PRIMARY KEY,
    "project_id" TEXT NOT NULL REFERENCES "projects"("id") ON DELETE CASCADE,
    "number" INTEGER NOT NULL,
    "parent_work_item_id" TEXT,
    "milestone_id" TEXT,
    "kind" TEXT NOT NULL,
    "title" TEXT NOT NULL,
    "description" TEXT NOT NULL,
    "status" TEXT NOT NULL DEFAULT ('backlog'),
    "priority" TEXT NOT NULL DEFAULT ('normal'),
    "estimate" INTEGER,
    "assignee_principal_type" TEXT,
    "assignee_principal_id" TEXT,
    "repo" TEXT,
    "version" INTEGER NOT NULL DEFAULT (1),
    "position" INTEGER NOT NULL DEFAULT (0),
    "archived_at" INTEGER,
    "created_at" INTEGER NOT NULL DEFAULT (unixepoch() * 1000),
    "updated_at" INTEGER NOT NULL DEFAULT (unixepoch() * 1000),
    FOREIGN KEY ("milestone_id", "project_id") REFERENCES "project_milestones" ("id", "project_id"),
    FOREIGN KEY ("parent_work_item_id", "project_id") REFERENCES "project_work_items" ("id", "project_id")
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "project_work_packets" (
    "id" TEXT PRIMARY KEY,
    "project_id" TEXT NOT NULL REFERENCES "projects"("id") ON DELETE CASCADE,
    "execution_id" TEXT NOT NULL,
    "work_item_id" TEXT NOT NULL,
    "plan_version" INTEGER NOT NULL,
    "work_item_version" INTEGER NOT NULL,
    "engine_query_id" TEXT NOT NULL,
    "citations" TEXT NOT NULL DEFAULT ('[]'),
    "requester_principal_type" TEXT NOT NULL,
    "requester_principal_id" TEXT NOT NULL,
    "created_at" INTEGER NOT NULL DEFAULT (unixepoch() * 1000),
    FOREIGN KEY ("execution_id", "project_id") REFERENCES "project_executions" ("id", "project_id") ON DELETE CASCADE,
    FOREIGN KEY ("work_item_id", "project_id") REFERENCES "project_work_items" ("id", "project_id") ON DELETE CASCADE
);

--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "projects" (
    "id" TEXT PRIMARY KEY,
    "slug" TEXT NOT NULL,
    "key_prefix" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "outcome" TEXT NOT NULL,
    "constraints" TEXT NOT NULL DEFAULT ('[]'),
    "non_goals" TEXT NOT NULL DEFAULT ('[]'),
    "status" TEXT NOT NULL DEFAULT ('draft'),
    "health" TEXT NOT NULL DEFAULT ('unknown'),
    "lead_principal_type" TEXT NOT NULL,
    "lead_principal_id" TEXT NOT NULL,
    "target_date" INTEGER,
    "current_plan_version" INTEGER NOT NULL DEFAULT (0),
    "version" INTEGER NOT NULL DEFAULT (1),
    "completion_policy" TEXT NOT NULL DEFAULT ('manual'),
    "creator_principal_type" TEXT NOT NULL,
    "creator_principal_id" TEXT NOT NULL,
    "created_at" INTEGER NOT NULL DEFAULT (unixepoch() * 1000),
    "updated_at" INTEGER NOT NULL DEFAULT (unixepoch() * 1000),
    "archived_at" INTEGER
);

--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "project_activity_events_project_created_idx" ON "project_activity_events" ("project_id", "created_at", "id");

--> statement-breakpoint

CREATE UNIQUE INDEX IF NOT EXISTS "project_approvals_proposal_uidx" ON "project_approvals" ("proposal_id");

--> statement-breakpoint

CREATE UNIQUE INDEX IF NOT EXISTS "project_command_receipts_principal_key_uidx" ON "project_command_receipts" ("principal_type", "principal_id", "idempotency_key");

--> statement-breakpoint

CREATE UNIQUE INDEX IF NOT EXISTS "project_executions_id_project_uidx" ON "project_executions" ("id", "project_id");

--> statement-breakpoint

CREATE UNIQUE INDEX IF NOT EXISTS "project_milestones_id_project_uidx" ON "project_milestones" ("id", "project_id");

--> statement-breakpoint

CREATE UNIQUE INDEX IF NOT EXISTS "project_work_items_project_number_uidx" ON "project_work_items" ("project_id", "number");

--> statement-breakpoint

CREATE UNIQUE INDEX IF NOT EXISTS "project_work_items_id_project_uidx" ON "project_work_items" ("id", "project_id");

--> statement-breakpoint

CREATE UNIQUE INDEX IF NOT EXISTS "projects_slug_uidx" ON "projects" ("slug");

--> statement-breakpoint

CREATE UNIQUE INDEX IF NOT EXISTS "projects_key_prefix_uidx" ON "projects" ("key_prefix");