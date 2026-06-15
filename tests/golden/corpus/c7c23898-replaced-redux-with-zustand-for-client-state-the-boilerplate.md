---
id: c7c23898
kind: decision
repo: lumen-web
tags:
- state
- redux
- zustand
author: ''
created: 2026-06-15T02:13:46.320113000Z
quality: 3
schema: 1
content_hash: c7c238983af0bbbc5b8328d24228f6023d72938dd977ca66fd3046d229341b66
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Replaced Redux with Zustand for client state. The boilerplate of actions, reducers, and thunks was not paying for itself in a mostly server-state app; Zustand's hook-based store plus React Query for server cache cut the state code roughly in half.