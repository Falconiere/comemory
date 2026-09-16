-- v18: drop the code references `cross_link` minted from scheme-prefixed and
-- dot-relative paths — `file:/tmp/check.db`, `sqlite:./data.db`,
-- `file:../../packages/database/.data/local.db` (issue #153).
--
-- Until this release `graph::cross_link::extract_refs` guarded only against
-- `://` and `@`, so a single-slash URL scheme or a relative path expression
-- written in prose matched the `<repo>:<path>` citation shape with the scheme
-- as the "repo". Such an edge can never resolve — no repo is named `file` —
-- and it sat in every context bundle that surfaced the memory as an
-- `unpinned` reference with no snippet. The extractor now refuses a captured
-- path that begins with `/`, `./` or `../`; this migration removes what the
-- old rule already wrote, in the three places a reference lives: the `edges`
-- row, its `code_ref` save-time anchor, and its `edge_fts` triplet (whose
-- UNINDEXED payload columns mirror the edge, so the same predicate applies).
--
-- A real `<repo>:<path>` citation is repo-relative and never starts with a
-- slash or a dot segment, so no legitimate reference matches. The markdown is
-- untouched: `comemory rebuild` re-extracts under the new rule and reaches
-- the same state.
--
-- The path is everything after the first `:`. `instr` returns 0 for a
-- `dst_id` with no colon at all, which would make `substr(..., 1, N)` judge
-- the whole id by its own first characters — so a colon is required
-- explicitly: a reference without a `<repo>:` half is not this shape and is
-- left alone.
--
-- Destructive class: it deletes existing rows, so a failed pre-migration
-- snapshot must refuse the upgrade rather than warn.

DELETE FROM edges
 WHERE src_kind = 'memory'
   AND rel IN ('references_file', 'references_symbol')
   AND instr(dst_id, ':') > 0
   AND (substr(dst_id, instr(dst_id, ':') + 1, 1) = '/'
     OR substr(dst_id, instr(dst_id, ':') + 1, 2) = './'
     OR substr(dst_id, instr(dst_id, ':') + 1, 3) = '../');

DELETE FROM code_ref
 WHERE instr(dst_id, ':') > 0
   AND (substr(dst_id, instr(dst_id, ':') + 1, 1) = '/'
     OR substr(dst_id, instr(dst_id, ':') + 1, 2) = './'
     OR substr(dst_id, instr(dst_id, ':') + 1, 3) = '../');

DELETE FROM edge_fts
 WHERE src_kind = 'memory'
   AND rel IN ('references_file', 'references_symbol')
   AND instr(dst_id, ':') > 0
   AND (substr(dst_id, instr(dst_id, ':') + 1, 1) = '/'
     OR substr(dst_id, instr(dst_id, ':') + 1, 2) = './'
     OR substr(dst_id, instr(dst_id, ':') + 1, 3) = '../');
