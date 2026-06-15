---
id: 8ef38cb3
kind: convention
repo: shipfast-api
tags:
- errors
- api
- http
author: ''
created: 2026-06-15T02:13:46.135461000Z
quality: 3
schema: 1
content_hash: 8ef38cb356a3752b27f15cd221fe238f742564f9bbc847c3943ea70a1e721f64
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
All API handlers return a typed AppError that maps to an RFC 7807 problem+json body. Never return a bare 500 string. The error enum carries an HTTP status and a stable machine-readable code so the frontend can branch on code, not on the human message.