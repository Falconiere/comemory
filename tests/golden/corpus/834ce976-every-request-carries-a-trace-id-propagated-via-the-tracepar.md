---
id: 834ce976
kind: convention
repo: shipfast-api
tags:
- logging
- tracing
- context
author: ''
created: 2026-06-15T02:13:46.222241000Z
quality: 3
schema: 1
content_hash: 834ce976f21e25bc48430c5d1e6f4c1bf6b27947a53817baa9d116866939e662
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Every request carries a trace_id propagated via the traceparent header and injected into the tracing span. Log lines are structured JSON with the trace_id field so we can pivot from a Grafana trace to all correlated logs without grep.