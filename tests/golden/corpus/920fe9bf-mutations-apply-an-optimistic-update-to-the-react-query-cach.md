---
id: 920fe9bf
kind: pattern
repo: lumen-web
tags:
- optimistic
- mutation
- rollback
author: ''
created: 2026-06-15T02:13:46.430841000Z
quality: 3
schema: 1
content_hash: 920fe9bfaf3dac207e828b323f995fcb1209d5101559cae0eaf54a5de3168e3e
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Mutations apply an optimistic update to the React Query cache immediately, then roll back to the snapshot in onError. The UI feels instant for likes and toggles while still reconciling with the server's authoritative response on settle.