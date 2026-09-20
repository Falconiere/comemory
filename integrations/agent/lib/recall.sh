#!/usr/bin/env bash
# shellcheck shell=bash
# Recall-injection and Stop-time advisory helpers for memory-lifecycle.sh.
# Split out once the hook grew past ~120 lines. Assumes the caller already
# sourced project-skills.sh (for ps_cfg_int/ps_cfg_bool/ps_config_root and,
# through it, comemory_repo_key) and that jq is present — both are guarded by
# the hook before it sources this file.

# recall_find_json REPO K PROMPT — a repo-scoped, untracked `find` over the
# memory domain, bounded to 5s when timeout/gtimeout is available (bare
# otherwise, matching every other bounded call in this plugin). Prints raw
# JSON on success; prints nothing and returns non-zero on any failure
# (missing comemory, timeout, non-zero exit). COMEMORY_DISABLE_ACCESS_TRACKING
# keeps the hint from minting its own retrieval_log row — the agent's own
# `find`/`search` call is the tracked one.
recall_find_json() {
  local repo="$1" k="$2" prompt="$3" bound=""
  if command -v timeout >/dev/null 2>&1; then bound="timeout 5"
  elif command -v gtimeout >/dev/null 2>&1; then bound="gtimeout 5"; fi
  # $bound is a plain string split on purpose (empty, or "timeout 5"), the
  # same shape as comemory-status.sh's bounded call.
  COMEMORY_DISABLE_ACCESS_TRACKING=true $bound \
    comemory find --repo "$repo" --domain memory --k "$k" --json -- "$prompt" 2>/dev/null
}

# recall_truncate_chars STR N — cap STR at N CHARACTERS, not bytes, so a
# multibyte title is never split mid-character. `${str:0:n}` is
# character-oriented in bash only under a multibyte-aware LC_CTYPE; printf's
# `%.Ns` is always byte-oriented regardless of locale — confirmed on this
# Mac's bash 3.2: `printf '%.1s' "über"` emits one lone continuation byte
# (invalid UTF-8), while `${s:0:1}` keeps the full two-byte character. Scope
# a UTF-8 LC_CTYPE to the slice alone (via `local`, restored on return) when
# the ambient one is not already UTF-8-flavoured and a UTF-8 locale is
# installed; otherwise fall back to whatever the ambient locale gives —
# still capped, just possibly byte-oriented at the very edge.
recall_truncate_chars() {
  local str="$1" n="$2" pick
  # LC_ALL outranks LC_CTYPE, so an ambient LC_ALL=C would silently turn the
  # slice back into bytes: read it for the UTF-8 check, then blank it for the
  # duration of this function so the LC_CTYPE below is the one that counts.
  local ambient_all="${LC_ALL:-}"
  local LC_ALL=""
  local LC_CTYPE="${LC_CTYPE:-}"
  case "$LC_CTYPE" in
    *[Uu][Tt][Ff]-8|*[Uu][Tt][Ff]8) ;;
    *)
      case "$ambient_all${LANG:-}" in
        *[Uu][Tt][Ff]-8|*[Uu][Tt][Ff]8) ;;
        *)
          pick=$(locale -a 2>/dev/null | grep -Eim1 '^(en_US|C)\.utf-?8$') || pick=""
          [ -n "$pick" ] && LC_CTYPE="$pick"
          ;;
      esac
      ;;
  esac
  printf '%s' "${str:0:$n}"
}

# recall_hint_from_json JSON — render `find --json` hits as "id  title"
# lines plus a closing instruction, capped at 4000 characters (see
# recall_truncate_chars above). Returns non-zero with no output on
# empty/unparseable JSON or zero hits, so the caller falls back to the
# standing reminder text.
recall_hint_from_json() {
  local json="$1" lines n ctx
  [ -n "$json" ] || return 1
  n=$(jq -r '(.hits // []) | length' <<<"$json" 2>/dev/null) || return 1
  [ "$n" -gt 0 ] 2>/dev/null || return 1
  lines=$(jq -r '(.hits // [])[] | (.id // "?") + "  " + (.title // .subtitle // "")' <<<"$json" 2>/dev/null) || return 1
  [ -n "$lines" ] || return 1
  ctx=$(printf 'Recall hint — possibly relevant memories:\n%s\nThese IDs came from an untracked find; selectively show only useful ones. Feedback applies only to a separate tracked recall with a query_id.' "$lines")
  recall_truncate_chars "$ctx" 4000
}

# recall_advisory INPUT_JSON CWD — Stop-time repository-window advisory.
# Prints one compact systemMessage and returns 0 when the shared repo window
# has judgeable pending recalls with neither a verdict nor a save. It never
# claims session ownership and never blocks. Returns 1 with no output for all
# skip cases. recall.enforce remains the compatibility switch. Empty
# returned_ids rows are omitted because no honest per-memory verdict can be
# recorded for them.
recall_advisory() {
  local input="$1" cwd="$2"
  local root session_id stop_active start_marker advised_marker since repo status
  local actionable pending fb saves ids
  [ "$(ps_cfg_bool recall.enforce true)" = true ] || return 1
  session_id=$(jq -r '.session_id // empty' <<<"$input" 2>/dev/null) || return 1
  # Sanitised: an untrusted session_id with '/' or '..' must not escape
  # $root/comemory via the marker file names built from it below.
  session_id=$(ps_sanitize_session_id "$session_id") || return 1
  stop_active=$(jq -r '.stop_hook_active // false' <<<"$input" 2>/dev/null) || return 1
  [ "$stop_active" != true ] || return 1
  command -v comemory >/dev/null 2>&1 || return 1
  root="$(ps_config_root)/comemory"
  start_marker="$root/session-$session_id.start"
  [ -e "$start_marker" ] || return 1
  advised_marker="$root/session-$session_id.advised"
  [ ! -e "$advised_marker" ] || return 1
  since=$(cat "$start_marker" 2>/dev/null) || return 1
  [ -n "$since" ] || return 1
  repo=$(comemory_repo_key "$cwd")
  [ -n "$repo" ] || return 1
  status=$(comemory recall-status --repo "$repo" --since "$since" --json 2>/dev/null) || return 1
  [ -n "$status" ] || return 1
  actionable=$(jq -c '[(.pending // [])[] | select((.returned_ids // []) | length > 0)]' \
    <<<"$status" 2>/dev/null) || return 1
  pending=$(jq -r 'length' <<<"$actionable" 2>/dev/null) || return 1
  fb=$(jq -r '.feedback_events // 0' <<<"$status" 2>/dev/null) || return 1
  saves=$(jq -r '.saves // 0' <<<"$status" 2>/dev/null) || return 1
  [ "$pending" -gt 0 ] 2>/dev/null || return 1
  [ "$fb" -eq 0 ] 2>/dev/null || return 1
  [ "$saves" -eq 0 ] 2>/dev/null || return 1
  ids=$(jq -r '[.[0:3][].query_id] | join(", ")' <<<"$actionable" 2>/dev/null) || ids=""
  mkdir -p "$root" 2>/dev/null || true
  # noclobber makes the check-and-create atomic when duplicate Stop hooks race.
  (set -o noclobber; : >"$advised_marker") 2>/dev/null || return 1
  jq -nc --argjson n "$pending" --arg ids "$ids" '
    {systemMessage:("Comemory: shared repository activity in this session window has " +
      ($n|tostring) + " unjudged recall(s) (e.g. query IDs: " + $ids +
      "). If useful, inspect recall_status and selectively show returned IDs; feedback only " +
      "what you judged. No action is required for work from another session.")}
  ' 2>/dev/null || true
  return 0
}
