-- One project with a row in every project-owned table, every nullable
-- column set in at least one row, written through real SQLite with foreign
-- keys enforced. Shared by the rebuild copy test (`tests/cli__rebuild_5.rs`)
-- and the transfer tests (#342).
--
-- `w-2` nests under `w-1` and `e-1` is superseded by `e-2`, so both
-- self-references travel; `e-2` is inserted first, so a row-at-a-time copy in
-- primary-key order must defer its keys. The creator is the local operator,
-- the lead another user and the agent a project agent, so an actor remap has
-- pairs to rewrite and pairs to leave alone. The receipt and the binding are
-- the two tables a transfer bundle never carries.
INSERT INTO projects (id, slug, key_prefix, name, outcome, constraints, non_goals, status,
    health, lead_principal_type, lead_principal_id, target_date, current_plan_version, version,
    completion_policy, creator_principal_type, creator_principal_id, created_at, updated_at,
    archived_at)
VALUES ('0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f', 'alpha', 'ALP', 'Alpha', 'ship alpha',
    '["no downtime"]', '["mobile"]', 'active', 'on_track', 'user', 'u-lead', 1767225600000, 2, 7,
    'manual', 'user', 'local-operator', 1759000000000, 1759000000500, 1759000000900);
INSERT INTO project_repositories (project_id, repo, creator_principal_type, creator_principal_id,
    created_at)
VALUES ('0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f', 'Falconiere/comemory', 'user', 'local-operator',
    1759000001000);
INSERT INTO project_milestones (id, project_id, name, description, target_date, position, status,
    archived_at)
VALUES ('m-1', '0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f', 'Beta', 'first cut', 1767225600000, 3,
    'active', 1759000002000);
INSERT INTO project_work_items (id, project_id, number, parent_work_item_id, milestone_id, kind,
    title, description, status, priority, estimate, assignee_principal_type,
    assignee_principal_id, repo, version, position, archived_at, created_at, updated_at)
VALUES ('w-1', '0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f', 1, NULL, 'm-1', 'feature', 'root',
        'the root item', 'in_progress', 'high', 5, 'user', 'local-operator',
        'Falconiere/comemory', 2, 1, NULL, 1759000003000, 1759000003100),
       ('w-2', '0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f', 2, 'w-1', 'm-1', 'task', 'child',
        'nested item', 'blocked', 'urgent', 3, 'project_agent', 'g-1', 'Falconiere/comemory', 4,
        2, 1759000003900, 1759000003200, 1759000003300);
INSERT INTO project_work_item_dependencies (project_id, blocker_id, blocked_id)
VALUES ('0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f', 'w-1', 'w-2');
INSERT INTO project_criteria (id, project_id, work_item_id, description, required,
    evidence_requirement, resolution, resolution_rationale, resolver_principal_type,
    resolver_principal_id, position, archived_at)
VALUES ('c-1', '0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f', 'w-2', 'tests pass', 0, 'verified',
    'waived', 'covered upstream', 'user', 'u-lead', 4, 1759000004000);
INSERT INTO project_executions (id, project_id, work_item_id, actor_principal_type,
    actor_principal_id, state, started_at, heartbeat_at, finished_at, result_summary, version,
    superseded_by_execution_id)
VALUES ('e-2', '0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f', 'w-2', 'project_agent', 'g-1', 'finished',
        1759000005000, 1759000005100, 1759000005200, 'resumed and done', 3, NULL),
       ('e-1', '0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f', 'w-2', 'project_agent', 'g-1', 'canceled',
        1759000004500, 1759000004600, 1759000004700, 'went stale', 2, 'e-2');
INSERT INTO project_work_packets (id, project_id, execution_id, work_item_id, plan_version,
    work_item_version, engine_query_id, citations, requester_principal_type,
    requester_principal_id, created_at)
VALUES ('k-1', '0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f', 'e-2', 'w-2', 2, 4, 'q-20260928-abc',
    '{"queryId":"q","items":[]}', 'project_agent', 'g-1', 1759000005300);
INSERT INTO project_plan_proposals (id, project_id, base_plan_version, state, operations,
    assumptions, risks, rationale, proposer_principal_type, proposer_principal_id,
    request_digest, created_at, updated_at)
VALUES ('r-1', '0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f', 1, 'approved', '[{"op":"add"}]', '["a"]',
    '["r"]', 'grow scope', 'project_agent', 'g-1', 'sha256:aa', 1759000006000, 1759000006100);
INSERT INTO project_approvals (id, project_id, proposal_id, decision, reviewer_principal_type,
    reviewer_principal_id, rationale, created_at)
VALUES ('ap-1', '0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f', 'r-1', 'approve', 'user',
    'local-operator', 'looks right', 1759000006200);
INSERT INTO project_evidence (id, project_id, work_item_id, execution_id, kind, source,
    external_id, url, trust, metadata, content_hash, verified_by, verified_at,
    creator_principal_type, creator_principal_id, created_at)
VALUES ('v-1', '0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f', 'w-2', 'e-2', 'pull_request', 'github',
    '325', 'https://github.com/Falconiere/comemory/pull/325', 'verified', '{"checks":"green"}',
    'sha256:bb', 'engine', 1759000007100, 'project_agent', 'g-1', 1759000007000);
INSERT INTO project_evidence_criteria (evidence_id, criterion_id) VALUES ('v-1', 'c-1');
INSERT INTO project_activity_events (id, project_id, actor_principal_type, actor_principal_id,
    event_type, entity_type, entity_id, payload, created_at)
VALUES ('ev-1', '0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f', 'user', 'local-operator',
    'proposal.approved', 'proposal', 'r-1', '{"version":2}', 1759000008000);
INSERT INTO project_command_receipts (id, project_id, principal_type, principal_id,
    idempotency_key, command_type, request_digest, response, created_at)
VALUES ('rc-1', '0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f', 'user', 'u-lead', 'key-1',
    'proposal.review', 'sha256:cc', '{"status":200}', 1759000009000);
INSERT INTO project_transfer_bindings (project_id, direction, remote, digest,
    remapped_from_principal_type, remapped_from_principal_id, transferred_at)
VALUES ('0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f', 'imported', 'ws-origin',
    'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd', 'user', 'u-before',
    1759000010000);
