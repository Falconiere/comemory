---
id: 4e45fa9d
kind: bug
repo: billing-svc
tags:
- billing
- notifications
- incident
author: ''
created: 2026-06-15T02:13:46.903155000Z
quality: 3
schema: 1
content_hash: 4e45fa9d77c321b8fc60a57f5abb3261b67dbc450703afc0096fc42ae96a86b7
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from:
  - 94633d83
---
A support escalation surfaced one customer receiving three duplicate copies of the same invoice notification within a minute. Replaying the sweep journal showed two shifts had each walked the pending notice cursor and neither saw the other's markers until the window closed, so billing-svc:src/notify/dispatcher.rs:send_invoice_notice fired once per shift per row. Throttling the mailer would have hidden the symptom without addressing the overlap.