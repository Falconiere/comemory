---
id: 302ae4ba
kind: decision
repo: shipfast-api
tags:
- postgres
- uuid
- primary-key
author: ''
created: 2026-06-15T02:13:46.121567000Z
quality: 3
schema: 1
content_hash: 302ae4ba8cd4dc9f25c8242101d894886105a0a2faaad04a7f7151c2af002c9d
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Decided to use UUIDv7 for primary keys instead of bigserial. UUIDv7 keeps the time-ordered prefix so b-tree index locality stays good while letting clients mint ids offline. We generate them in the app layer and never expose internal sequence counts to the public API.