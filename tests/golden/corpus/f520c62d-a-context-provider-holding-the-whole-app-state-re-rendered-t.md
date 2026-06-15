---
id: f520c62d
kind: discovery
repo: lumen-web
tags:
- rerender
- memo
- context
author: ''
created: 2026-06-15T02:13:46.417124000Z
quality: 3
schema: 1
content_hash: f520c62d09df9215655bce6ae746a21f19c88e252affde15139b30ebf8a83581
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
A context provider holding the whole app state re-rendered the entire tree on every keystroke in the search box. Split the context into a stable dispatch context and a value context, and memoized leaf components; typing jank disappeared.