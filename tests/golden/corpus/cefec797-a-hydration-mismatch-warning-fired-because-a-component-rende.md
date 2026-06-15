---
id: cefec797
kind: bug
repo: lumen-web
tags:
- hydration
- ssr
- nextjs
author: ''
created: 2026-06-15T02:13:46.374766000Z
quality: 3
schema: 1
content_hash: cefec797388b683512b30a52c334c0106614062d7a010afc2de954eb593b66dc
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
A hydration mismatch warning fired because a component rendered new Date().toLocaleString() on both server and client, producing different strings. Fixed by rendering the timestamp only after mount with a useEffect, leaving the SSR markup deterministic.