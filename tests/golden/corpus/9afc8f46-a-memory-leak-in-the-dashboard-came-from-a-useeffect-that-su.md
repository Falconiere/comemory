---
id: 9afc8f46
kind: bug
repo: lumen-web
tags:
- react
- useeffect
- memory-leak
author: ''
created: 2026-06-15T02:13:46.306666000Z
quality: 3
schema: 1
content_hash: 9afc8f46fcce1d7380329e0e21955a16ce80901dbad1fce8427b6bb6ca676c6c
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
A memory leak in the dashboard came from a useEffect that subscribed to a websocket but returned no cleanup function, so each route change stacked another listener. Added the unsubscribe in the effect cleanup and guarded setState with an isMounted ref.