-- Rebuild "sync_exchange": SQLite cannot alter a CHECK in place. The
-- network state gains 'restore_unverified' (#256): a key held by an
-- unverified restore, its upstream's or this engine's own.
PRAGMA foreign_keys = OFF;
--> statement-breakpoint
CREATE TABLE "_toolu_new_sync_exchange" (
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
    "network_state" TEXT NOT NULL DEFAULT ('ok') CHECK (network_state IN ('ok', 'backoff', 'auth_suspended', 'protocol_error', 'restore_unverified')),
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
INSERT INTO "_toolu_new_sync_exchange" ("api_url", "workspace_id", "protocol", "coverage_reason", "selected_at", "upgrade_through", "replay_kind", "replay_state", "replay_scan_through", "replay_target", "network_state", "retry_at", "consecutive_failures", "last_error", "suspended_fingerprint", "upstream_head", "stall_sequence", "stall_reason", "last_session_at", "last_ok_at", "updated_at") SELECT "api_url", "workspace_id", "protocol", "coverage_reason", "selected_at", "upgrade_through", "replay_kind", "replay_state", "replay_scan_through", "replay_target", "network_state", "retry_at", "consecutive_failures", "last_error", "suspended_fingerprint", "upstream_head", "stall_sequence", "stall_reason", "last_session_at", "last_ok_at", "updated_at" FROM "sync_exchange";
--> statement-breakpoint
DROP TABLE "sync_exchange";
--> statement-breakpoint
PRAGMA legacy_alter_table = ON;
--> statement-breakpoint
ALTER TABLE "_toolu_new_sync_exchange" RENAME TO "sync_exchange";
--> statement-breakpoint
PRAGMA legacy_alter_table = OFF;
