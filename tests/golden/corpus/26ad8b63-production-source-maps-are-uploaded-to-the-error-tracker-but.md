---
id: 26ad8b63
kind: note
repo: lumen-web
tags:
- webpack
- sourcemap
- debug
author: ''
created: 2026-06-15T02:13:46.472534000Z
quality: 3
schema: 1
content_hash: 26ad8b630b4773f718cb806ff2df4c061d8a55d59b12ca96466c7870766c4cae
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Production source maps are uploaded to the error tracker but not served to clients. The build emits hidden-source-map so stack traces in Sentry resolve to original TS lines while end users cannot download the maps from the CDN.