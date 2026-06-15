---
id: 2bbfbe7e
kind: bug
repo: shipfast-api
tags:
- redis
- cache
- ttl
author: ''
created: 2026-06-15T02:13:46.149023000Z
quality: 3
schema: 1
content_hash: 2bbfbe7ea2de8eae2b615ebbceff15d619c163a1b277ed0917512901022cd205
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Cache stampede took down the pricing service: when the hot key expired, thousands of requests recomputed the same value simultaneously. Fixed with a probabilistic early-expiration (XFetch) jitter so one request refreshes the key slightly before TTL while the rest serve the stale value.