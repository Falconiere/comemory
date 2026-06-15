---
id: 496f00d6
kind: discovery
repo: shipfast-api
tags:
- postgres
- index
- query-plan
author: ''
created: 2026-06-15T02:13:46.486492000Z
quality: 3
schema: 1
content_hash: 496f00d6f0999a6ed7506bbefc1645e9ae9372a10d93105f17b8aacd2d752d05
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
A slow report query ignored the composite index because the leading column was wrapped in lower(). Created an expression index on lower(email) and the planner switched from a seq scan to an index scan; query went from 4s to 12ms.