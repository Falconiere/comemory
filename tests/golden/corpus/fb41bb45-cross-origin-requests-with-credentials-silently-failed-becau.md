---
id: fb41bb45
kind: bug
repo: lumen-web
tags:
- cors
- preflight
- credentials
author: ''
created: 2026-06-15T02:13:46.444772000Z
quality: 3
schema: 1
content_hash: fb41bb45eb8af50e15dc80b14d4f52842cdfb4c737afc355f4ccabd0156923cb
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Cross-origin requests with credentials silently failed because the server reflected Access-Control-Allow-Origin as a wildcard, which browsers reject when credentials are included. Set the header to the exact origin and added Access-Control-Allow-Credentials true.