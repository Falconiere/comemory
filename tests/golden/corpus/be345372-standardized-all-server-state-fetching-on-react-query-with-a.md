---
id: be345372
kind: decision
repo: lumen-web
tags:
- fetch
- react-query
- cache
author: ''
created: 2026-06-15T02:13:46.389055000Z
quality: 3
schema: 1
content_hash: be345372b504cba45ce829f38593e4833d7bb6e186857ccec949f8956e396889
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Standardized all server-state fetching on React Query with a 30 second stale time and background refetch on window focus. We deleted the bespoke useFetch hooks; cache invalidation now goes through query keys instead of manual state juggling.