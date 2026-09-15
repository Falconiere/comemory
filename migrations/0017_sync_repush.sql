-- v17: re-offer every local memory to the organization exactly once.
--
-- Until `2026-09-14-sync-everything-realtime-design.md` the push path dropped
-- any memory with an empty `repo` label, and `run_push` advanced `pushed_seq`
-- past it anyway. Those entries therefore sit BEHIND the cursor: removing the
-- filter alone would never re-offer them, and a memory saved outside a git
-- worktree would stay stranded forever on an upgraded machine.
--
-- Resetting the cursor re-walks the whole outbox once. That is safe and cheap:
-- the platform answers `exists` for anything it already holds, and the walk is
-- bounded by the same batch loop a first login uses. `pulled_seq` is left
-- alone — nothing was ever wrongly pulled.
--
-- Destructive class: it rewrites existing rows in place, so a failed
-- pre-migration snapshot must refuse the upgrade rather than warn.

UPDATE sync_state SET pushed_seq = 0;
