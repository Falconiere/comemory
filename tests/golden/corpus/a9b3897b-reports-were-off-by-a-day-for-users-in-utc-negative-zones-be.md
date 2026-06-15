---
id: a9b3897b
kind: bug
repo: shipfast-api
tags:
- timezone
- datetime
- utc
author: ''
created: 2026-06-15T02:13:46.278877000Z
quality: 3
schema: 1
content_hash: a9b3897bcda4fe5c5598a2849aaab7aec471ae7029278e93e49126e73a0f2ef4
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Reports were off by a day for users in UTC-negative zones because we stored local timestamps without an offset. Migrated all timestamp columns to timestamptz, normalized existing data to UTC, and now render in the user's zone only at the presentation layer.