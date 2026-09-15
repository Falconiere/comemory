#!/usr/bin/env bash
# comemory CLI wrapper — persistent memory for AI agents
# Defaults to the current project (auto-detected via comemory_repo_key) and
# scopes every operation to it via comemory's server-side --repo filter.
# Override the repo with TOOLU_COMEMORY_REPO=<name>. The historical
# MY_CLAUDE_COMEMORY_REPO name remains a lower-precedence compatibility alias.
# Honors COMEMORY_DATA_DIR (passed through to the comemory CLI, which already
# reads it from the environment) for the data root.
set -euo pipefail

# Real directory of THIS script, with symlinks resolved. BASH_SOURCE holds the
# path as INVOKED, and register.sh publishes this wrapper as a SYMLINK at
# $CLAUDE_CONFIG_DIR/comemory/comemory.sh — the path SKILL.md and the scope-hook
# deny banner both tell agents to use. Through that link ${BASH_SOURCE%/*} is
# the LINK's dir, so every sibling-relative path below resolved against a tree
# that does not exist: the repo-scope lib silently went missing and every memory
# landed in the shared "unknown" pool. Walk the link chain by hand (readlink -f
# is not portable to older BSD/macOS) so paths resolve against the real tree.
_self="${BASH_SOURCE[0]}"
# Guarantee a directory component: for a bare `comemory.sh` invocation
# `${_self%/*}` yields the FILENAME, not a dir, so both the relative-link join
# and the final cd would resolve against nonsense.
case "$_self" in */*) ;; *) _self="./$_self" ;; esac
# Belt-and-braces hop cap. A true cycle is not reachable through BASH_SOURCE[0]
# — the kernel already walked the chain to execute this file, so an ELOOP would
# have failed the invocation before any of this ran — but an unbounded `while`
# over attacker- or bug-supplied link data is not worth leaving in. 40 matches
# the usual SYMLOOP_MAX; on hitting it, stop resolving and let the missing-lib
# branch below report a broken install rather than spinning.
_hops=0
while [ -L "$_self" ]; do
  if [ "$_hops" -ge 40 ]; then
    printf 'comemory.sh: symlink chain exceeds 40 hops at %s — refusing to follow further\n' "$_self" >&2
    break
  fi
  _hops=$((_hops + 1))
  _link=$(readlink "$_self")
  case "$_link" in
    /*) _self="$_link" ;;
    *)  _self="${_self%/*}/$_link" ;;
  esac
done
_dir=$(cd "${_self%/*}" 2>/dev/null && pwd) || _dir="."

# Canonical, worktree-shared repo scope via the plugin's own repo-scope lib (a
# sibling that ships with comemory — no cross-plugin path, unlike toolu's
# detect.sh which resolves the per-worktree --show-toplevel for gate state).
# comemory_repo_key returns 0 even when empty so `set -e` reaches the
# REPO="unknown" fallback below. Missing lib → noop key → "unknown"; _rs_ok
# records which of those two causes fired so the warning below can name it.
_rs="$_dir/../../../lib/repo-scope.sh"
# shellcheck source=../../../lib/repo-scope.sh
if [ -r "$_rs" ]; then . "$_rs"; _rs_ok=1; else comemory_repo_key() { :; }; _rs_ok=0; fi

# No comemory CLI? Graceful no-op so dependent skills don't break — but warn on
# stderr first so the agent SEES that this operation (e.g. a save) was dropped,
# not silently swallowed mid-session.
if ! command -v comemory >/dev/null 2>&1; then
  printf 'comemory.sh: comemory CLI not installed — "%s" skipped (no-op). Install comemory to persist/recall.\n' "${1:-<subcommand>}" >&2
  exit 0
fi

REPO="${TOOLU_COMEMORY_REPO:-${MY_CLAUDE_COMEMORY_REPO:-$(comemory_repo_key)}}"
if [ -z "$REPO" ]; then
  REPO="unknown"
  # Visibility: an empty key means every memory lands in the shared "unknown"
  # pool, silently co-mingling across runs. Warn once so the contamination is
  # not invisible — and name the ACTUAL cause. A missing repo-scope lib produces
  # the same empty key as being outside a repo, and blaming git for a broken
  # install sends the reader hunting the wrong thing.
  if [ "$_rs_ok" -eq 0 ]; then
    printf 'comemory.sh: repo-scope lib not found at %s — cannot detect the repo, scoping to "unknown" (broken install; set TOOLU_COMEMORY_REPO to isolate)\n' "$_rs" >&2
  else
    printf 'comemory.sh: no git repo and no repo override — scoping to "unknown" (set TOOLU_COMEMORY_REPO to isolate)\n' >&2
  fi
fi
# A flag-like repo value (leading '-') would be parsed by comemory/clap as a
# flag rather than the --repo argument. Refuse it — fall back to "unknown".
case "$REPO" in
  -*)
    printf 'comemory.sh: ignoring flag-like repo name "%s" — scoping to "unknown"\n' "$REPO" >&2
    REPO="unknown" ;;
esac

# Inject `--repo "$REPO"` UNLESS the caller already passed --repo: a second
# --repo would clap-collide on a duplicate single-value flag (the same hazard
# the `summary` verb guards for --tags). Sets the REPO_ARGS array; expand it
# set-u-safe with ${REPO_ARGS[@]+"${REPO_ARGS[@]}"} (empty-array-safe on bash 3.2).
repo_flag() {
  case " $* " in
    *" --repo "*|*" --repo="*) REPO_ARGS=() ;;
    *)                          REPO_ARGS=(--repo "$REPO") ;;
  esac
}

# True when a captured `search` result holds no memories. Branches on output mode:
# --json carries an explicit "total":0 / "hits":[] (a hit's "total":N never matches
# ":0" exactly — "total":10 has ":1"); plain output prints one score row per hit
# (`0.99  lexical  <id>`) above a `query:`/`feedback:` trailer, so empty == no line
# begins with a digit once ANSI colour is stripped (trailers begin with a letter).
# Args: $1 = captured stdout, $2 = "json" when --json was passed.
_search_is_empty() {
  local out="$1" mode="$2"
  if [ "$mode" = json ]; then
    case "$out" in
      *'"hits":[]'*|*'"total":0'*) return 0 ;;
      *)                           return 1 ;;
    esac
  fi
  if printf '%s\n' "$out" | sed $'s/\x1b\\[[0-9;]*m//g' | grep -qE '^[[:space:]]*[0-9]'; then
    return 1
  fi
  return 0
}

# Print the miss -> save-back nudge. STDERR ONLY so --json stdout stays pure.
# Conditional wording (IF reusable) — a miss can legitimately mean "nothing to do".
# Args: $1 = query, $2 = repo scope.
_emit_miss_banner() {
  {
    printf 'comemory.sh: no memory hit for "%s" (repo: %s).\n' "$1" "$2"
    printf '  IF you learn something reusable answering this (pattern/bug/decision/nuance), save it back:\n'
    printf '    comemory.sh save "<title>" "<body>" --kind pattern|bug|decision|discovery|note --tags "..."\n'
    printf '  (the wrapper auto-injects --repo %s — scope is handled).\n' "$2"
  } >&2
}

subcmd="${1:-}"
shift 2>/dev/null || true

usage() {
  cat <<'USAGE'
Usage: comemory.sh <subcommand> [args...]

Memory (repo-scoped — --repo auto-injected):
  search <query> [flags]              Search memories (--k N to widen; --kind to filter)
  save <title> <content> [flags]      Save a memory (--kind defaults to note)
  context <query> [flags]             Headline lookup: code symbol + memories matching a key
  list [flags]                        List memories
  delete <id> [flags]                 Soft-delete a memory by 8-hex id (moves to .trash/)
  summary <content>                   Save a session summary (tags: session-summary; yields to caller --tags)

Code intelligence (repo-scoped — --repo auto-injected):
  search-code <query> [flags]         Lexical code search (--lang, --k). NOTE: semantic ranking
                                      needs an embedder comemory does not ship; without one this
                                      is FTS/BM25 only — prefer ast-grep for structural queries.
  index-code --path <dir> [flags]     Index a repo's code symbols (lexical). --path required.
  graph [flags]                       Code relationship graph (--rel imports|co-changed|all,
                                      --format json|dot|html, --min-weight N)

Retrieval-quality loop (GLOBAL — no --repo; local, no LLM/API):
  feedback <query_id> [flags]         Record recall relevance (--used/--irrelevant <csv ids>)
  mine [--apply]                      Mine query-expansion pairs from the retrieval log
  tune [--apply]                      Grid-search ranking blend weights against a golden set
  eval [flags]                        Score recall@k + MRR (--golden <file>, --k N)
  prune [--apply]                     Soft-delete low-value memories + orphan edges
  gc                                  Hard-delete trashed entries + stale telemetry
  rebuild                             Rebuild the SQLite mirror from markdown source of truth
  maintain                            Autonomous upkeep: mine --apply + prune --apply + gc.
                                      Each step is bounded by timeout/gtimeout when present; on a
                                      host with neither (stock macOS) a hung comemory can block a
                                      manual call — the session-end hook runs it detached instead.
  stats                               Data-directory + index health report (comemory doctor)

Pass-through flags: --kind <kind>, --tags <csv>, --quality N, --k N, --lang <lang>, --json
USAGE
  exit 1
}

# Positional values (query, save body) are passed AFTER a `--` end-of-options
# marker so a value with a leading `--` (e.g. a title/query that starts with
# "--foo") is parsed as the positional, not mistaken for a flag. comemory/clap
# requires every flag BEFORE the `--`, so the order is: <verb> <flags> -- <value>.
case "$subcmd" in
  search)
    query="${1:?search requires a query}"
    shift
    repo_flag "$@"
    # Capture stdout (not exec) so an EMPTY result can nudge a save-back — closing
    # the miss -> native-search -> save loop. comemory's own stderr inherits ours
    # (real errors stay visible); only stdout is captured and re-emitted verbatim.
    # set +e around the capture: under `set -e`/pipefail a non-zero comemory must
    # not abort the wrapper before we forward its exit code. The banner fires only
    # on success (rc 0) AND empty — never masking an error as a miss.
    _json_mode=""
    case " $* " in *" --json "*|*" --json="*) _json_mode=json ;; esac
    set +e
    _out=$(comemory search ${REPO_ARGS[@]+"${REPO_ARGS[@]}"} "$@" -- "$query")
    _rc=$?
    set -e
    printf '%s\n' "$_out"
    if [ "$_rc" -eq 0 ] && _search_is_empty "$_out" "$_json_mode"; then
      _emit_miss_banner "$query" "$REPO"
    fi
    exit "$_rc"
    ;;
  context)
    # Headline lookup (code symbol + memories). Repo-scoped like search: comemory
    # accepts an optional --repo, so auto-inject it and pass the query positional
    # after the `--` end-of-options marker.
    query="${1:?context requires a query}"
    shift
    repo_flag "$@"
    exec comemory context ${REPO_ARGS[@]+"${REPO_ARGS[@]}"} "$@" -- "$query"
    ;;
  save)
    title="${1:?save requires a title}"
    content="${2:?save requires content}"
    shift 2
    repo_flag "$@"
    exec comemory save ${REPO_ARGS[@]+"${REPO_ARGS[@]}"} "$@" -- "$(printf '%s\n\n%s' "$title" "$content")"
    ;;
  list)
    repo_flag "$@"
    exec comemory list ${REPO_ARGS[@]+"${REPO_ARGS[@]}"} "$@"
    ;;
  delete)
    # Soft-delete by id. The 8-hex id is globally unique, so `comemory delete`
    # takes NO --repo — dispatch like the retrieval-loop verbs (no auto-inject),
    # with the id passed as the positional after `--`.
    id="${1:?delete requires a memory id (8-hex, from search/list)}"
    shift
    exec comemory delete "$@" -- "$id"
    ;;
  summary)
    content="${1:?summary requires content}"
    shift
    # Stamp the title with a UTC timestamp so repeated summaries are not
    # title-identical (a fixed "Session summary" title makes comemory's
    # near-duplicate auto-warn fire on every save). Fall back to epoch seconds
    # if formatted `date` yields nothing, so the de-dup stamp is never empty.
    stamp="$(date -u +%Y-%m-%dT%H:%MZ 2>/dev/null)"
    [ -n "$stamp" ] || stamp="$(date +%s 2>/dev/null)"
    [ -n "$stamp" ] || stamp="${EPOCHSECONDS:-0}"
    body="$(printf 'Session summary %s\n\n%s' "$stamp" "$content")"
    repo_flag "$@"
    # Default the session-summary tag, but yield to a caller-supplied --tags:
    # comemory/clap rejects a duplicate single-value flag. --kind is left to
    # comemory's own default (note) so a caller may override it via "$@".
    case " $* " in
      *" --tags "*|*" --tags="*)
        exec comemory save ${REPO_ARGS[@]+"${REPO_ARGS[@]}"} "$@" -- "$body" ;;
      *)
        exec comemory save --tags session-summary ${REPO_ARGS[@]+"${REPO_ARGS[@]}"} "$@" -- "$body" ;;
    esac
    ;;
  stats)
    # No REPO_ARGS: `doctor` is a global command (data-dir/index health), not
    # repo-scoped — matching the retrieval-loop verbs below. Do not add --repo.
    exec comemory doctor "$@"
    ;;

  # ── Code intelligence (repo-scoped) ──────────────────────────────────
  # Lexical only without an embedder; comemory ships none. ast-grep remains
  # first choice for structural queries — see SKILL.md.
  search-code)
    query="${1:?search-code requires a query}"
    shift
    repo_flag "$@"
    exec comemory search-code ${REPO_ARGS[@]+"${REPO_ARGS[@]}"} "$@" -- "$query"
    ;;
  index-code)
    # --path is required by comemory; caller supplies it. --repo auto-injected
    # unless the caller passed their own.
    repo_flag "$@"
    exec comemory index-code ${REPO_ARGS[@]+"${REPO_ARGS[@]}"} "$@"
    ;;
  graph)
    repo_flag "$@"
    exec comemory graph ${REPO_ARGS[@]+"${REPO_ARGS[@]}"} "$@"
    ;;

  # ── Retrieval-quality loop (GLOBAL — comemory has no --repo on these) ──
  # All local, no LLM/API. Safe to run autonomously (see `maintain`).
  feedback)
    query_id="${1:?feedback requires a query_id (from a prior search --json)}"
    shift
    exec comemory feedback "$@" -- "$query_id"
    ;;
  mine|tune|eval|prune|gc|rebuild)
    exec comemory "$subcmd" "$@"
    ;;
  maintain)
    # Autonomous upkeep bundle. Each step is best-effort and non-fatal so a
    # failure in one never blocks the others. Each is bounded by timeout/gtimeout
    # when available (matching session-end.sh) so a hung step can't block a
    # manual `mod.sh comemory maintain`; bare on hosts with neither.
    # This verb is hand-run only — the session-end hook runs its own detached
    # mine/prune/gc sequence and never dispatches here. So keep stderr VISIBLE
    # (a real failure like a non-writable data dir must surface to the operator);
    # only stdout is silenced, and each step stays non-fatal so one failure never
    # blocks the others.
    _cm_to=""
    if command -v timeout >/dev/null 2>&1; then _cm_to="timeout 30"
    elif command -v gtimeout >/dev/null 2>&1; then _cm_to="gtimeout 30"; fi
    $_cm_to comemory mine --apply >/dev/null || true
    $_cm_to comemory prune --apply >/dev/null || true
    $_cm_to comemory gc >/dev/null || true
    ;;

  *)
    usage
    ;;
esac
