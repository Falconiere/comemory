---
id: e4ac8713
kind: decision
repo: shipfast-api
tags:
- kafka
- outbox
- events
author: ''
created: 2026-06-15T02:13:46.193064000Z
quality: 3
schema: 1
content_hash: e4ac87135afdaa98f9fc2b9d01f8dc2f9f2816eaa357fc92e2a00d4b28c342f7
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Adopted the transactional outbox pattern for publishing domain events. The event row is written in the same DB transaction as the state change, then a relay polls the outbox and pushes to Kafka. This removes the dual-write inconsistency between the database and the event bus.