---
id: e901902d
kind: bug
repo: shipfast-api
tags:
- migration
- deadlock
- alter-table
author: ''
created: 2026-06-15T02:13:46.207840000Z
quality: 3
schema: 1
content_hash: e901902ddf5ce163ca56bcf575bdaa75dfc806c748e14b3f53e36472c4489726
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
A zero-downtime migration deadlocked because ALTER TABLE took an ACCESS EXCLUSIVE lock while a long analytics query held a competing lock. Fixed by setting lock_timeout to 2s in the migration session and retrying with backoff, so the migration yields instead of blocking writes.