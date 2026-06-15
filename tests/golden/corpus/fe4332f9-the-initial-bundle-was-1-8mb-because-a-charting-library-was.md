---
id: fe4332f9
kind: discovery
repo: lumen-web
tags:
- bundle
- code-split
- lazy
author: ''
created: 2026-06-15T02:13:46.347630000Z
quality: 3
schema: 1
content_hash: fe4332f9c5b298f04266e103392ebf5eed8a440a34acedadee37e8e0fe1f6ceb
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
The initial bundle was 1.8MB because a charting library was imported at the app root. Lazy-loaded it behind a dynamic import on the analytics route only; first-contentful-paint improved by 1.2s and the main chunk shrank to 480KB.