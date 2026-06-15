---
id: afd1bd61
kind: pattern
repo: shipfast-api
tags:
- retry
- backoff
- circuit-breaker
author: ''
created: 2026-06-15T02:13:46.500634000Z
quality: 3
schema: 1
content_hash: afd1bd614141dfb452722212499cab15b452e3cbd2e76e5f0b649a2e67f63426
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Outbound calls to third-party APIs wrap a circuit breaker around exponential backoff with full jitter. After five consecutive failures the breaker opens for 30s and fast-fails, shedding load from the struggling dependency instead of piling on retries.