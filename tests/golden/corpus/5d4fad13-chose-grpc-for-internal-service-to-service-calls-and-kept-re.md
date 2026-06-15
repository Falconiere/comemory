---
id: 5d4fad13
kind: decision
repo: shipfast-api
tags:
- grpc
- rest
- api-gateway
author: ''
created: 2026-06-15T02:13:46.265410000Z
quality: 3
schema: 1
content_hash: 5d4fad13bf74eabcd9f4aa8e6acf65bc1e4ec7d71e80605c54c5bea95fb535cc
references:
  symbols: []
  files: []
relations:
  supersedes: []
  conflicts_with: []
  derived_from: []
---
Chose gRPC for internal service-to-service calls and kept REST only at the public gateway edge. Internal protobuf contracts give us codegen, streaming, and smaller payloads, while the gateway translates the public REST surface into gRPC.