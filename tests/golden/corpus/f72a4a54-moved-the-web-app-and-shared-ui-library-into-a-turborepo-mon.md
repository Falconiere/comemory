---
id: f72a4a54
kind: decision
repo: lumen-web
tags:
- monorepo
- turborepo
- packages
author: ''
created: 2026-06-15T02:13:46.458455000Z
quality: 3
schema: 1
content_hash: f72a4a541060c6ca64a14f814a31efb2d39e09e8f4936680449ed94688ab4551
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Moved the web app and shared UI library into a Turborepo monorepo with a shared tsconfig and a single ESLint config package. Remote caching cut CI build time by 60 percent because unchanged packages are restored from cache instead of rebuilt.