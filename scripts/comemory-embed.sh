#!/usr/bin/env bash
# Bridges comemory ↔ Ollama for the BYO-vector flow.
# Usage:
#   comemory-embed save --kind decision "body text"
#   comemory-embed search "query"
set -euo pipefail
: "${COMEMORY_EMBED_URL:=http://localhost:11434/api/embeddings}"
: "${COMEMORY_EMBED_MODEL:=nomic-embed-text}"

embed() {
    local text="$1"
    curl -fsS "$COMEMORY_EMBED_URL" \
        -d "$(jq -n --arg m "$COMEMORY_EMBED_MODEL" --arg t "$text" \
              '{model:$m, prompt:$t}')" \
      | jq -c '{embedding}'
}

cmd="$1"; shift
case "$cmd" in
    save)
        body="${@: -1}"
        embed "$body" | comemory save --vector-stdin "$@" ;;
    search)
        query="$1"; shift
        embed "$query" | comemory search "$query" --vector-stdin "$@" ;;
    *) echo "usage: comemory-embed save|search ..."; exit 64 ;;
esac
