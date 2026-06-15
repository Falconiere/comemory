---
id: ea4c5335
kind: bug
repo: shipfast-api
tags:
- postgres
- pgbouncer
- pool
author: ''
created: 2026-06-15T02:13:46.107124000Z
quality: 3
schema: 1
content_hash: ea4c53357f683966e5ae8d04dcfec68195099dad12f72b478d3d3d01c39bcffa
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Postgres connection pool was exhausted under load because pgbouncer ran in session pooling mode, holding a backend connection for the entire client session. Switched pgbouncer to transaction mode and dropped default_pool_size to 20; idle backends now return to the pool between transactions and the exhaustion alerts stopped.