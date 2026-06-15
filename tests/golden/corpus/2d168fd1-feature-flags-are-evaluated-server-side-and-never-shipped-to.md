---
id: 2d168fd1
kind: convention
repo: shipfast-api
tags:
- feature-flag
- rollout
- config
author: ''
created: 2026-06-15T02:13:46.292908000Z
quality: 3
schema: 1
content_hash: 2d168fd1a646b3bfa0f9edc53e7c59c2a86b28ab64c23f400c518baa88011ba2
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Feature flags are evaluated server-side and never shipped to the client as raw booleans. Each flag has an owner, an expiry date, and a kill switch. Stale flags past expiry fail CI so the codebase does not accumulate dead branches.