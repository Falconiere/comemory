---
id: d3fe2052
kind: discovery
repo: shipfast-api
tags:
- jwt
- auth
- clock-skew
author: ''
created: 2026-06-15T02:13:46.163878000Z
quality: 3
schema: 1
content_hash: d3fe2052405762330a299a5758fb46a37863adfe1c60641ed9b2dc0c71afe4b5
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Discovered intermittent 401s were caused by clock skew between auth nodes: tokens issued on a node a few seconds ahead failed nbf validation on a lagging node. Added a 30 second leeway to the JWT validator and enabled chrony NTP sync across the fleet.