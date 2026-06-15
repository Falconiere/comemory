---
id: 8eee499b
kind: pattern
repo: shipfast-api
tags:
- rate-limit
- token-bucket
- redis
author: ''
created: 2026-06-15T02:13:46.251219000Z
quality: 3
schema: 1
content_hash: 8eee499ba0d8a2270f06e00e22d3d5600db1a7cab586efa476efaece305a70f3
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Rate limiting uses a Redis-backed token bucket implemented as a Lua script so the check-and-decrement is atomic. The bucket key includes the API key and the route class; refill rate and burst are configured per plan tier.