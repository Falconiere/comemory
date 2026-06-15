---
id: a1bf18f9
kind: pattern
repo: shipfast-api
tags:
- idempotency
- payments
- stripe
author: ''
created: 2026-06-15T02:13:46.178326000Z
quality: 3
schema: 1
content_hash: a1bf18f94c8d5ebdfcc0d64b1dc3535ecb7f9f9384da3534aa38843e0e24a02e
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Payment endpoints use an idempotency-key table keyed on the client-supplied key plus the request hash. A repeated key with a matching hash replays the stored response; a mismatched hash returns 409. This makes Stripe charge retries safe under network partitions.