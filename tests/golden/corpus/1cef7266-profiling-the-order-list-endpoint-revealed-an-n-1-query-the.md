---
id: 1cef7266
kind: discovery
repo: shipfast-api
tags:
- n+1
- orm
- eager-load
author: ''
created: 2026-06-15T02:13:46.237053000Z
quality: 3
schema: 1
content_hash: 1cef72666713c30d031b25ea09f072a4611015a2737695940f16ec2f3f24438d
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Profiling the order list endpoint revealed an N+1 query: the serializer lazily loaded each order's line items. Switched to an eager join with a single query batched by order id; p95 latency on that route dropped from 800ms to 90ms.