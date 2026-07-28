---
id: 9823d92d
kind: decision
repo: billing-svc
tags:
- billing
- notifications
- dedupe
author: ''
created: 2026-06-15T02:13:46.902144000Z
quality: 3
schema: 1
content_hash: 9823d92dc35e345a11da14bbc898b65ef6c798e7202aeb43577e9fbd127e29ed
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from:
  - 94633d83
---
Customers received duplicate invoice notifications whenever two sweeps overlapped, so the mailer now claims each notice before dispatch. A marker row naming the invoice and its template is recorded in the same transaction that enqueues the notice, and a claim that already exists short-circuits the dispatch. See billing-svc:src/notify/dispatcher.rs:send_invoice_notice.