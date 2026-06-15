---
id: afa955e3
kind: convention
repo: lumen-web
tags:
- css
- tailwind
- design-tokens
author: ''
created: 2026-06-15T02:13:46.334365000Z
quality: 3
schema: 1
content_hash: afa955e39c9cf268a2ef5bcf36fa792f0489597eafc63f59eb9f73271c9e8f8f
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Spacing, color, and typography come exclusively from design tokens exposed as Tailwind theme keys. No raw hex values or arbitrary pixel margins in components. A lint rule rejects arbitrary Tailwind values like p-[13px] so the design system stays the single source of truth.