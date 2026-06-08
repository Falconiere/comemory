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

cmd="${1:-}"
if [[ -z "$cmd" ]]; then
    echo "usage: comemory-embed save|search ..." >&2
    exit 64
fi
shift
command -v curl >/dev/null || { echo "comemory-embed requires curl" >&2; exit 69; }
command -v jq   >/dev/null || { echo "comemory-embed requires jq"   >&2; exit 69; }
case "$cmd" in
    save)
        # `${@: -1:1}` is `set -u`-safe even when $@ is empty; the inner guard
        # rejects the case where the would-be body is empty or looks like a
        # flag (the caller forgot the positional body argument).
        body="${*: -1:1}"
        if [[ -z "$body" || "$body" == -* ]]; then
            echo "usage: comemory-embed save [opts] BODY" >&2
            exit 64
        fi
        embed "$body" | comemory save --vector-stdin "$@" ;;
    search)
        # `${1:-}` keeps `set -u` from killing the script before the usage
        # line on a `comemory-embed search` with no query argument.
        query="${1:-}"
        if [[ -z "$query" ]]; then
            echo "usage: comemory-embed search QUERY [opts]" >&2
            exit 64
        fi
        shift
        embed "$query" | comemory search "$query" --vector-stdin "$@" ;;
    *) echo "usage: comemory-embed save|search ..." >&2; exit 64 ;;
esac
