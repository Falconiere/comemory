---
id: 3f22189d
kind: pattern
repo: lumen-web
tags:
- forms
- validation
- zod
author: ''
created: 2026-06-15T02:13:46.361234000Z
quality: 3
schema: 1
content_hash: 3f22189d41588bc263ebc276cbf9c902d5a809966702430f0126f9d1ea48957c
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Forms share a single Zod schema between the client validator and the server handler. The same schema drives field-level error messages, the submit guard, and the API request type, so client and server can never disagree on what a valid payload looks like.