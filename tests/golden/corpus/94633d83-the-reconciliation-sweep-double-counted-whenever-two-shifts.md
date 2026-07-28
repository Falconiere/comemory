---
id: 94633d83
kind: discovery
repo: billing-svc
tags:
- reconciliation
- batch
- concurrency
author: ''
created: 2026-06-15T02:13:46.901133000Z
quality: 3
schema: 1
content_hash: 94633d837e2cbb31f9b750f294e07adb3b91c81ca6c9704ed49c3645a5b45f8d
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
The reconciliation sweep double-counted whenever two shifts of the nightly job overlapped. Each shift claimed a marker row at startup, but the marker was refreshed on a ticker that shared a mutex with the ledger flush, so a slow flush let the marker go stale and a standby shift took it over mid-sweep. Refreshing the marker from a dedicated fiber that never touches the flush mutex removed the overlap.