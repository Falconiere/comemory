#!/usr/bin/env bash
# shellcheck shell=bash
# Configuration, paths, metadata, and usage-store helpers for project skills.

ps_config_root() {
  if [ -n "${TOOLU_CONFIG_DIR:-}" ]; then printf '%s' "$TOOLU_CONFIG_DIR"
  elif [ "${TOOLU_HOST_OVERRIDE:-}" = codex ] || { [ -z "${TOOLU_HOST_OVERRIDE:-}" ] && [ -n "${PLUGIN_ROOT:-}" ]; }; then printf '%s' "${CODEX_HOME:-$HOME/.codex}"
  else printf '%s' "${CLAUDE_CONFIG_DIR:-$HOME/.claude}"; fi
}

ps_project_dirname() {
  if [ -n "${TOOLU_PROJECT_CONFIG_DIRNAME:-}" ]; then printf '%s' "$TOOLU_PROJECT_CONFIG_DIRNAME"
  elif [ "${TOOLU_HOST_OVERRIDE:-}" = codex ] || { [ -z "${TOOLU_HOST_OVERRIDE:-}" ] && [ -n "${PLUGIN_ROOT:-}" ]; }; then printf '.codex'
  else printf '.claude'; fi
}

ps_repo_root() {
  local dir="${1:-.}" top
  top=$(git -C "$dir" rev-parse --show-toplevel 2>/dev/null || true)
  [ -n "$top" ] && printf '%s' "$top"
  return 0
}

ps_now() {
  python3 -c 'from datetime import datetime, timezone; print(datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"))' 2>/dev/null && return 0
  printf '1970-01-01T00:00:00Z'
}

ps_realpath() {
  local p="$1" out
  out=$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$p" 2>/dev/null) && { printf '%s' "$out"; return 0; }
  out=$(readlink -f "$p" 2>/dev/null) && { printf '%s' "$out"; return 0; }
  return 1
}

ps_now_epoch() { date -u +%s 2>/dev/null || printf '0'; }
ps_iso_to_epoch() {
  local iso="$1" e
  e=$(date -u -d "$iso" +%s 2>/dev/null) && { printf '%s' "$e"; return 0; }
  e=$(date -u -j -f "%Y-%m-%dT%H:%M:%SZ" "$iso" +%s 2>/dev/null) && { printf '%s' "$e"; return 0; }
  return 1
}
ps_skills_dir() { printf '%s/.toolu/skills' "$1"; }
ps_usage_path() { printf '%s/.usage.json' "$(ps_skills_dir "$1")"; }

PS_CFG_JSON=''
PS_CFG_LOADED=0
ps_load_cfg() {
  [ "$PS_CFG_LOADED" = 1 ] && return 0
  PS_CFG_LOADED=1; PS_CFG_JSON='{}'
  command -v jq >/dev/null 2>&1 || return 0
  local user_cfg project_cfg user_json='{}' project_json='{}' root
  user_cfg="$(ps_config_root)/comemory.json"; root=$(ps_repo_root "${PS_CWD:-.}")
  [ -n "$root" ] && project_cfg="$root/$(ps_project_dirname)/comemory.json"
  [ -f "$user_cfg" ] && user_json=$(jq -e . "$user_cfg" 2>/dev/null) || user_json='{}'
  if [ -n "${project_cfg:-}" ] && [ -f "$project_cfg" ]; then project_json=$(jq -e . "$project_cfg" 2>/dev/null) || project_json='{}'; fi
  PS_CFG_JSON=$(jq -cn --argjson u "$user_json" --argjson p "$project_json" '$u * $p' 2>/dev/null) || PS_CFG_JSON='{}'
}
ps_memory_enabled() { ps_load_cfg; command -v jq >/dev/null 2>&1 && jq -e '.skills.comemory != false' <<<"$PS_CFG_JSON" >/dev/null; }
ps_loop_enabled() { ps_memory_enabled && jq -e '.projectSkills.enabled != false' <<<"$PS_CFG_JSON" >/dev/null; }
ps_cfg_int() {
  local path="$1" def="$2" typ val
  ps_load_cfg; command -v jq >/dev/null 2>&1 || { printf '%s' "$def"; return 0; }
  typ=$(jq -r --arg p "$path" 'getpath($p | split(".")) | type' <<<"$PS_CFG_JSON" 2>/dev/null || echo "null")
  val=$(jq -r --arg p "$path" 'getpath($p | split(".")) // empty' <<<"$PS_CFG_JSON" 2>/dev/null || true)
  case "$typ" in number) case "$val" in ''|*[!0-9]*) printf '%s' "$def" ;; *) printf '%s' "$val" ;; esac ;; null) printf '%s' "$def" ;; *) printf 'project-skills: %s is not an integer; using %s\n' "$path" "$def" >&2; printf '%s' "$def" ;; esac
}
ps_thresholds() {
  PS_STALE=$(ps_cfg_int projectSkills.staleAfterDays 30); PS_ARCHIVE=$(ps_cfg_int projectSkills.archiveAfterDays 90); PS_INDEX_CAP=$(ps_cfg_int projectSkills.indexCap 20)
  if [ "$PS_STALE" -gt "$PS_ARCHIVE" ] 2>/dev/null; then printf 'project-skills: staleAfterDays > archiveAfterDays; using 30/90\n' >&2; PS_STALE=30; PS_ARCHIVE=90; fi
  [ "$PS_INDEX_CAP" -ge 1 ] 2>/dev/null || PS_INDEX_CAP=20
}

ps_load_usage() { [ -f "$1" ] && jq -e . "$1" 2>/dev/null || printf '{}'; }
ps_save_usage() {
  local path="$1" json="$2" dir tmp
  dir=$(dirname "$path"); mkdir -p "$dir" || return 1; tmp="$path.tmp.$$"
  if printf '%s\n' "$json" >"$tmp" 2>/dev/null && mv -f "$tmp" "$path" 2>/dev/null; then return 0; fi
  rm -f "$tmp" 2>/dev/null; return 1
}
ps_valid_name() {
  case "$1" in ''|*[!a-z0-9-]*|-*|*-) return 1 ;; esac
  printf '%s' "$1" | grep -qE '^[a-z][a-z0-9-]{0,63}$'
}
ps_word_count() { printf '%s' "$1" | wc -w | tr -d ' '; }
ps_strip_frontmatter() { awk 'BEGIN { fm=0 } NR==1 && $0=="---" { fm=1; next } fm && $0=="---" { fm=0; next } !fm { print }'; }
ps_has_required_headings() {
  local body="$1"
  printf '%s\n' "$body" | tr -d '\r' | grep -q '^## When to Use$' || return 1
  printf '%s\n' "$body" | tr -d '\r' | grep -q '^## Procedure$' || return 1
  printf '%s\n' "$body" | tr -d '\r' | grep -q '^## Pitfalls$' || return 1
  printf '%s\n' "$body" | tr -d '\r' | grep -q '^## Verification$'
}
ps_skill_origin() {
  [ -f "$1" ] || return 0
  awk 'BEGIN { fm=0 } $0=="---" { if (fm==0) { fm=1; next } else exit } fm && $0 ~ /^[[:space:]]*origin:[[:space:]]*/ { sub(/^[[:space:]]*origin:[[:space:]]*/, ""); gsub(/[[:space:]]+$/, ""); print; exit }' "$1"
}
ps_skill_description() {
  [ -f "$1" ] || return 0
  awk 'BEGIN { fm=0 } $0=="---" { if (fm==0) { fm=1; next } else exit } fm && $0 ~ /^description:[[:space:]]*/ { sub(/^description:[[:space:]]*/, ""); gsub(/^["'\'']|["'\'']$/, ""); print; exit }' "$1"
}
ps_render_skill() {
  local name="$1" desc="$2" origin="$3" created="$4" body="$5"
  cat <<EOF
---
name: $name
description: $desc
metadata:
  toolu:
    origin: $origin
    created: $created
---
$body
EOF
}
ps_ensure_tree() {
  local dir="$1"
  mkdir -p "$dir" || return 1
  [ -f "$dir/.gitignore" ] || printf '%s\n' '.archive/' '.usage.json' >"$dir/.gitignore" || return 1
  mkdir -p "$dir/.archive" || return 1
  # shellcheck disable=SC2016 # Markdown code spans are intentionally literal.
  [ -f "$dir/README.md" ] || printf '%s\n' '# Project skills' '' 'Agent-created procedures for this repo. Read a `SKILL.md` to load one. The curator archives unused *agent-created* skills into `.archive/`; it never deletes, and never touches marketplace plugin skills.' >"$dir/README.md"
}
ps_default_entry() {
  jq -nc --arg o "$1" --arg c "$2" --arg n "$3" '{origin:$o,created_at:$c,use_count:0,patch_count:0,last_used_at:null,last_patched_at:null,state:"active",pinned:false,first_seen_at:$n}'
}
ps_list_skill_names() {
  local dir="$1" d
  [ -d "$dir" ] || return 0
  for d in "$dir"/*/SKILL.md; do
    [ -f "$d" ] || continue
    basename "$(dirname "$d")"
  done
}
