-- A committed plan for project 11111111-1111-4111-8111-111111111111, written
-- straight into the store (no command writes a plan before #338). The project
-- row must already exist; this moves it to plan version 3 and adds:
--   milestones  a..01, a..02 (both position 0, so id breaks the tie) and the
--               archived a..03;
--   work items  b..01 #1 (position 1) in a..01; b..02 #2 nested under b..01
--               with every optional field set; the archived b..03 #3 in the
--               archived a..03; b..04 #4 with no milestone; b..05 #5
--               (position 2), live but in the archived a..03;
--   criteria    c..01 project-level; c..02 on b..02; the archived c..03 on
--               b..01; c..04, live but on the archived b..03;
--   edges       b..01 -> b..02 and b..04 -> b..02 are live; b..01 -> b..03 and
--               b..03 -> b..04 each name the archived item.
UPDATE projects SET current_plan_version = 3
 WHERE id = '11111111-1111-4111-8111-111111111111';

INSERT INTO project_milestones
  (id, project_id, name, description, target_date, position, status, archived_at)
VALUES
  ('a0000000-0000-4000-8000-000000000001', '11111111-1111-4111-8111-111111111111',
   'Alpha', 'The reader ships', 1790812800000, 0, 'planned', NULL),
  ('a0000000-0000-4000-8000-000000000002', '11111111-1111-4111-8111-111111111111',
   'Beta', 'Proposals ship', 1794700800000, 0, 'in_progress', NULL),
  ('a0000000-0000-4000-8000-000000000003', '11111111-1111-4111-8111-111111111111',
   'Dropped', 'Cut from scope', 1796083200000, 1, 'planned', 1790000000000);

INSERT INTO project_work_items
  (id, project_id, number, parent_work_item_id, milestone_id, kind, title,
   description, status, priority, estimate, assignee_principal_type,
   assignee_principal_id, repo, version, position, archived_at)
VALUES
  ('b0000000-0000-4000-8000-000000000001', '11111111-1111-4111-8111-111111111111',
   1, NULL, 'a0000000-0000-4000-8000-000000000001', 'task', 'Build the reader',
   'Read the plan', 'backlog', 'normal', NULL, NULL, NULL, NULL, 1, 1, NULL),
  ('b0000000-0000-4000-8000-000000000002', '11111111-1111-4111-8111-111111111111',
   2, 'b0000000-0000-4000-8000-000000000001', 'a0000000-0000-4000-8000-000000000001',
   'task', 'Render the items', 'Every live item', 'ready', 'high', 3, 'user',
   'local-operator', 'falconiere/comemory', 2, 0, NULL),
  ('b0000000-0000-4000-8000-000000000003', '11111111-1111-4111-8111-111111111111',
   3, NULL, 'a0000000-0000-4000-8000-000000000003', 'task', 'Dropped item',
   'Archived by an approved proposal', 'backlog', 'low', NULL, NULL, NULL, NULL,
   1, 0, 1790000000000),
  ('b0000000-0000-4000-8000-000000000004', '11111111-1111-4111-8111-111111111111',
   4, NULL, NULL, 'bug', 'Fix the edge', 'No milestone', 'backlog', 'normal',
   NULL, NULL, NULL, NULL, 1, 0, NULL),
  ('b0000000-0000-4000-8000-000000000005', '11111111-1111-4111-8111-111111111111',
   5, NULL, 'a0000000-0000-4000-8000-000000000003', 'task', 'Outlive the milestone',
   'Live in an archived milestone', 'backlog', 'normal', NULL, NULL, NULL, NULL,
   1, 2, NULL);

INSERT INTO project_criteria
  (id, project_id, work_item_id, description, required, evidence_requirement,
   resolution, resolution_rationale, position, archived_at)
VALUES
  ('c0000000-0000-4000-8000-000000000001', '11111111-1111-4111-8111-111111111111',
   NULL, 'Plans read offline', 1, 'reported', 'open', NULL, 0, NULL),
  ('c0000000-0000-4000-8000-000000000002', '11111111-1111-4111-8111-111111111111',
   'b0000000-0000-4000-8000-000000000002', 'Items render', 0, 'verified',
   'waived', 'Covered elsewhere', 1, NULL),
  ('c0000000-0000-4000-8000-000000000003', '11111111-1111-4111-8111-111111111111',
   'b0000000-0000-4000-8000-000000000001', 'Archived criterion', 1, 'reported',
   'open', NULL, 0, 1790000000000),
  ('c0000000-0000-4000-8000-000000000004', '11111111-1111-4111-8111-111111111111',
   'b0000000-0000-4000-8000-000000000003', 'Outlive the item', 1, 'reported',
   'open', NULL, 2, NULL);

INSERT INTO project_work_item_dependencies (project_id, blocker_id, blocked_id)
VALUES
  ('11111111-1111-4111-8111-111111111111', 'b0000000-0000-4000-8000-000000000004',
   'b0000000-0000-4000-8000-000000000002'),
  ('11111111-1111-4111-8111-111111111111', 'b0000000-0000-4000-8000-000000000001',
   'b0000000-0000-4000-8000-000000000003'),
  ('11111111-1111-4111-8111-111111111111', 'b0000000-0000-4000-8000-000000000003',
   'b0000000-0000-4000-8000-000000000004'),
  ('11111111-1111-4111-8111-111111111111', 'b0000000-0000-4000-8000-000000000001',
   'b0000000-0000-4000-8000-000000000002');
