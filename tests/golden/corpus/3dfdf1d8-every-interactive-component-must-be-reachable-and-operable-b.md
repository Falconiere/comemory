---
id: 3dfdf1d8
kind: convention
repo: lumen-web
tags:
- accessibility
- aria
- keyboard
author: ''
created: 2026-06-15T02:13:46.402961000Z
quality: 3
schema: 1
content_hash: 3dfdf1d8652a00cc835a16f5455d5c0ce7a4af7821438ac78b23f4b9a1c01a76
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Every interactive component must be reachable and operable by keyboard alone and expose an accessible name. Custom dropdowns and modals follow the WAI-ARIA authoring patterns for focus trapping and escape-to-close; CI runs axe against the storybook.