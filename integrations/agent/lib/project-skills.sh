#!/usr/bin/env bash
# shellcheck shell=bash
# Project-skills store: agent-created SKILL.md files under <repo>/.toolu/skills/.
# Sourced by skills.sh and the comemory plugin's index / use / curate hooks.
# No cross-plugin source; jq is required for usage.json and config.

# ps_config_root — host-native writable config dir (mirrors comemory hooks).
ps_config_root() {
  if [ -n "${TOOLU_CONFIG_DIR:-}" ]; then
    printf '%s' "$TOOLU_CONFIG_DIR"
  elif [ "${TOOLU_HOST_OVERRIDE:-}" = codex ] || { [ -z "${TOOLU_HOST_OVERRIDE:-}" ] && [ -n "${PLUGIN_ROOT:-}" ]; }; then
    printf '%s' "${CODEX_HOME:-$HOME/.codex}"
  else
    printf '%s' "${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
  fi
}

# ps_project_dirname — .claude | .codex (overridable).
ps_project_dirname() {
  if [ -n "${TOOLU_PROJECT_CONFIG_DIRNAME:-}" ]; then
    printf '%s' "$TOOLU_PROJECT_CONFIG_DIRNAME"
  elif [ "${TOOLU_HOST_OVERRIDE:-}" = codex ] || { [ -z "${TOOLU_HOST_OVERRIDE:-}" ] && [ -n "${PLUGIN_ROOT:-}" ]; }; then
    printf '.codex'
  else
    printf '.claude'
  fi
}

# ps_repo_root [dir] — git toplevel, or empty. Returns 0 always.
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

# Canonical path including a final symlink (SKILL.md → elsewhere).
ps_realpath() {
  local p="$1" out
  out=$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$p" 2>/dev/null) && {
    printf '%s' "$out"
    return 0
  }
  out=$(readlink -f "$p" 2>/dev/null) && {
    printf '%s' "$out"
    return 0
  }
  return 1
}

ps_now_epoch() { date -u +%s 2>/dev/null || printf '0'; }

# ps_iso_to_epoch ISO — GNU date first, BSD next. Empty on failure.
ps_iso_to_epoch() {
  local iso="$1" e
  e=$(date -u -d "$iso" +%s 2>/dev/null) && { printf '%s' "$e"; return 0; }
  e=$(date -u -j -f "%Y-%m-%dT%H:%M:%SZ" "$iso" +%s 2>/dev/null) && { printf '%s' "$e"; return 0; }
  return 1
}

ps_skills_dir() {
  local root="$1"
  printf '%s/.toolu/skills' "$root"
}

ps_usage_path() {
  printf '%s/.usage.json' "$(ps_skills_dir "$1")"
}

# ── config (subset of comemory.json, local so this plugin stays self-contained)
PS_CFG_JSON=''
PS_CFG_LOADED=0

ps_load_cfg() {
  [ "$PS_CFG_LOADED" = 1 ] && return 0
  PS_CFG_LOADED=1
  PS_CFG_JSON='{}'
  command -v jq >/dev/null 2>&1 || return 0
  local user_cfg project_cfg user_json='{}' project_json='{}' root
  user_cfg="$(ps_config_root)/comemory.json"
  root=$(ps_repo_root "${PS_CWD:-.}")
  if [ -n "$root" ]; then
    project_cfg="$root/$(ps_project_dirname)/comemory.json"
  fi
  if [ -f "$user_cfg" ]; then
    user_json=$(jq -e . "$user_cfg" 2>/dev/null) || user_json='{}'
  fi
  if [ -n "${project_cfg:-}" ] && [ -f "$project_cfg" ]; then
    project_json=$(jq -e . "$project_cfg" 2>/dev/null) || project_json='{}'
  fi
  PS_CFG_JSON=$(jq -cn --argjson u "$user_json" --argjson p "$project_json" '$u * $p' 2>/dev/null) || PS_CFG_JSON='{}'
}

# The memory switch is independent of the optional project-skill loop.
ps_memory_enabled() {
  ps_load_cfg
  command -v jq >/dev/null 2>&1 || return 1
  jq -e '.skills.comemory != false' <<<"$PS_CFG_JSON" >/dev/null
}

ps_loop_enabled() {
  ps_memory_enabled || return 1
  jq -e '.projectSkills.enabled != false' <<<"$PS_CFG_JSON" >/dev/null
}

# ps_cfg_int PATH DEFAULT — PATH is a jq path like projectSkills.staleAfterDays.
ps_cfg_int() {
  local path="$1" def="$2" typ val
  ps_load_cfg
  command -v jq >/dev/null 2>&1 || { printf '%s' "$def"; return 0; }
  typ=$(jq -r --arg p "$path" 'getpath($p | split(".")) | type' <<<"$PS_CFG_JSON" 2>/dev/null || echo "null")
  val=$(jq -r --arg p "$path" 'getpath($p | split(".")) // empty' <<<"$PS_CFG_JSON" 2>/dev/null || true)
  case "$typ" in
    number)
      case "$val" in
        ''|*[!0-9]*) printf '%s' "$def" ;;
        *) printf '%s' "$val" ;;
      esac
      ;;
    null) printf '%s' "$def" ;;
    *)
      printf 'project-skills: %s is not an integer; using %s\n' "$path" "$def" >&2
      printf '%s' "$def"
      ;;
  esac
}

ps_thresholds() {
  # sets PS_STALE PS_ARCHIVE PS_INDEX_CAP (globals for this process)
  PS_STALE=$(ps_cfg_int projectSkills.staleAfterDays 30)
  PS_ARCHIVE=$(ps_cfg_int projectSkills.archiveAfterDays 90)
  PS_INDEX_CAP=$(ps_cfg_int projectSkills.indexCap 20)
  if [ "$PS_STALE" -gt "$PS_ARCHIVE" ] 2>/dev/null; then
    printf 'project-skills: staleAfterDays > archiveAfterDays; using 30/90\n' >&2
    PS_STALE=30
    PS_ARCHIVE=90
  fi
  [ "$PS_INDEX_CAP" -ge 1 ] 2>/dev/null || PS_INDEX_CAP=20
}

# ── usage.json
ps_load_usage() {
  local path="$1"
  if [ -f "$path" ]; then
    jq -e . "$path" 2>/dev/null || printf '{}'
  else
    printf '{}'
  fi
}

# ps_save_usage PATH JSON — atomic tmp+mv.
ps_save_usage() {
  local path="$1" json="$2" dir tmp
  dir=$(dirname "$path")
  mkdir -p "$dir" || return 1
  tmp="$path.tmp.$$"
  if printf '%s\n' "$json" >"$tmp" 2>/dev/null && mv -f "$tmp" "$path" 2>/dev/null; then
    return 0
  fi
  rm -f "$tmp" 2>/dev/null
  return 1
}

# 1–64 chars: leading [a-z] plus 0–63 of [a-z0-9-].
ps_valid_name() {
  case "$1" in
    ''|*[!a-z0-9-]*|-*|*-) return 1 ;;
  esac
  printf '%s' "$1" | grep -qE '^[a-z][a-z0-9-]{0,63}$'
}

ps_word_count() {
  printf '%s' "$1" | wc -w | tr -d ' '
}

ps_strip_frontmatter() {
  awk '
    BEGIN { fm=0 }
    NR==1 && $0=="---" { fm=1; next }
    fm && $0=="---" { fm=0; next }
    !fm { print }
  '
}

ps_has_required_headings() {
  local body="$1"
  printf '%s\n' "$body" | tr -d '\r' | grep -q '^## When to Use$' || return 1
  printf '%s\n' "$body" | tr -d '\r' | grep -q '^## Procedure$' || return 1
  printf '%s\n' "$body" | tr -d '\r' | grep -q '^## Pitfalls$' || return 1
  printf '%s\n' "$body" | tr -d '\r' | grep -q '^## Verification$' || return 1
}

ps_skill_origin() {
  local file="$1"
  [ -f "$file" ] || return 0
  awk '
    BEGIN { fm=0 }
    $0=="---" { if (fm==0) { fm=1; next } else exit }
    fm && $0 ~ /^[[:space:]]*origin:[[:space:]]*/ {
      sub(/^[[:space:]]*origin:[[:space:]]*/, "")
      gsub(/[[:space:]]+$/, "")
      print
      exit
    }
  ' "$file"
}

ps_skill_description() {
  local file="$1"
  [ -f "$file" ] || return 0
  awk '
    BEGIN { fm=0 }
    $0=="---" { if (fm==0) { fm=1; next } else exit }
    fm && $0 ~ /^description:[[:space:]]*/ {
      sub(/^description:[[:space:]]*/, "")
      gsub(/^["'\'']|["'\'']$/, "")
      print
      exit
    }
  ' "$file"
}

# ps_render_skill NAME DESC ORIGIN CREATED BODY -> stdout SKILL.md
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
  if [ ! -f "$dir/.gitignore" ]; then
    printf '%s\n' '.archive/' '.usage.json' >"$dir/.gitignore" || return 1
  fi
  mkdir -p "$dir/.archive" || return 1
  if [ ! -f "$dir/README.md" ]; then
    printf '%s\n' \
      '# Project skills' \
      '' \
      'Agent-created procedures for this repo. Read a `SKILL.md` to load one. The curator archives unused *agent-created* skills into `.archive/`; it never deletes, and never touches marketplace plugin skills.' \
      >"$dir/README.md" || return 1
  fi
}

# ps_seed_usage ROOT JSON_VAR_NAME — not nameref (bash 3.2). Prints updated JSON.
ps_default_entry() {
  local origin="$1" created="$2" now="$3"
  jq -nc --arg o "$origin" --arg c "$created" --arg n "$now" \
    '{origin:$o,created_at:$c,use_count:0,patch_count:0,last_used_at:null,last_patched_at:null,state:"active",pinned:false,first_seen_at:$n}'
}

ps_list_skill_names() {
  local dir="$1" d
  [ -d "$dir" ] || return 0
  for d in "$dir"/*/SKILL.md; do
    [ -f "$d" ] || continue
    basename "$(dirname "$d")"
  done
}

ps_create() {
  local name="$1" desc="$2" src="$3"
  local root dir dest body created now usage path entry
  root=$(ps_repo_root "${PS_CWD:-.}")
  if [ -z "$root" ]; then
    printf 'skills.sh: not in a git repo\n' >&2
    return 1
  fi
  ps_valid_name "$name" || { printf 'skills.sh: invalid name "%s" (1–64 chars, [a-z][a-z0-9-]*)\n' "$name" >&2; return 1; }
  if [ "$(ps_word_count "$desc")" -gt 30 ]; then
    printf 'skills.sh: description must be ≤ 30 words\n' >&2
    return 1
  fi
  [ -n "$desc" ] || { printf 'skills.sh: --description is required\n' >&2; return 1; }
  if [ -n "$src" ]; then
    [ -f "$src" ] || { printf 'skills.sh: --file %s not found\n' "$src" >&2; return 1; }
    body=$(ps_strip_frontmatter <"$src")
  else
    body=$(ps_strip_frontmatter)
  fi
  # trim a single leading newline from stripped body
  body=$(printf '%s\n' "$body" | sed '1{/^$/d;}')
  ps_has_required_headings "$body" || {
    printf 'skills.sh: body must contain ## When to Use, ## Procedure, ## Pitfalls, ## Verification\n' >&2
    return 1
  }
  dir=$(ps_skills_dir "$root")
  dest="$dir/$name/SKILL.md"
  if [ -e "$dir/$name" ]; then
    printf 'skills.sh: skill "%s" already exists\n' "$name" >&2
    return 1
  fi
  ps_ensure_tree "$dir" || return 1
  mkdir -p "$dir/$name" || return 1
  created=$(ps_now)
  now=$created
  ps_render_skill "$name" "$desc" agent "$created" "$body" >"$dest" || return 1
  path=$(ps_usage_path "$root")
  usage=$(ps_load_usage "$path")
  entry=$(ps_default_entry agent "$created" "$now")
  usage=$(jq --arg n "$name" --argjson e "$entry" '.[$n]=$e' <<<"$usage") || return 1
  ps_save_usage "$path" "$usage" || return 1
  printf 'created %s\n' "$dest"
}

ps_list() {
  local json="$1"
  local root dir usage name origin state
  root=$(ps_repo_root "${PS_CWD:-.}")
  [ -n "$root" ] || { printf 'skills.sh: not in a git repo\n' >&2; return 1; }
  dir=$(ps_skills_dir "$root")
  usage=$(ps_load_usage "$(ps_usage_path "$root")")
  if [ "$json" = 1 ]; then
    jq -n --argjson u "$usage" '
      $u | with_entries(select(.value.state == "active" or .value.state == "stale"))
    '
    return 0
  fi
  for name in $(ps_list_skill_names "$dir"); do
    origin=$(ps_skill_origin "$dir/$name/SKILL.md")
    state=$(jq -r --arg n "$name" '.[$n].state // "active"' <<<"$usage")
    printf '%s\t%s\t%s\n' "$name" "${origin:-unmanaged}" "$state"
  done
}

ps_index() {
  local root dir usage cap name desc ts ranked tmp
  root=$(ps_repo_root "${PS_CWD:-.}")
  [ -n "$root" ] || return 0
  dir=$(ps_skills_dir "$root")
  [ -d "$dir" ] || return 0
  ps_thresholds
  cap=$PS_INDEX_CAP
  usage=$(ps_load_usage "$(ps_usage_path "$root")")
  tmp=$(mktemp) || return 0
  for name in $(ps_list_skill_names "$dir"); do
    [ -f "$dir/$name/SKILL.md" ] || continue
    ts=$(jq -r --arg n "$name" \
      '.[$n].last_used_at // .[$n].created_at // .[$n].first_seen_at // ""' \
      <<<"$usage" 2>/dev/null || true)
    [ "$ts" = "null" ] && ts=""
    printf '%s\t%s\n' "$ts" "$name"
  done | sort -r >"$tmp"
  ranked=$(awk -F '\t' 'NF>=2 {print $2}' "$tmp" | head -n "$cap" || true)
  rm -f "$tmp"
  [ -n "$ranked" ] || return 0
  while IFS= read -r name; do
    [ -n "$name" ] || continue
    desc=$(ps_skill_description "$dir/$name/SKILL.md")
    [ -n "$desc" ] || continue
    printf -- '- %s: %s\n' "$name" "$desc"
  done <<EOF
$ranked
EOF
}

ps_require_name() {
  local name="$1" root dir
  ps_valid_name "$name" || { printf 'skills.sh: invalid name "%s"\n' "$name" >&2; return 1; }
  root=$(ps_repo_root "${PS_CWD:-.}")
  [ -n "$root" ] || { printf 'skills.sh: not in a git repo\n' >&2; return 1; }
  dir=$(ps_skills_dir "$root")
  printf '%s\t%s' "$root" "$dir"
}

ps_archive() {
  local name="$1" dry="${2:-}"
  local pair root dir usage origin pinned dest
  pair=$(ps_require_name "$name") || return 1
  root=${pair%%	*}
  dir=${pair#*	}
  [ -d "$dir/$name" ] || { printf 'skills.sh: no skill "%s"\n' "$name" >&2; return 1; }
  usage=$(ps_load_usage "$(ps_usage_path "$root")")
  origin=$(jq -r --arg n "$name" '.[$n].origin // empty' <<<"$usage")
  [ -n "$origin" ] || origin=$(ps_skill_origin "$dir/$name/SKILL.md")
  [ "$origin" = "agent" ] || { printf 'skills.sh: "%s" is unmanaged (adopt first)\n' "$name" >&2; return 1; }
  pinned=$(jq -r --arg n "$name" '.[$n].pinned // false' <<<"$usage")
  [ "$pinned" = "true" ] && { printf 'skills.sh: "%s" is pinned\n' "$name" >&2; return 1; }
  dest="$(ps_skills_dir "$root")/.archive/$name"
  if [ "$dry" = 1 ]; then
    printf 'would archive %s -> %s\n' "$name" "$dest"
    return 0
  fi
  mkdir -p "$(dirname "$dest")" || return 1
  [ ! -e "$dest" ] || { printf 'skills.sh: archive already has "%s"\n' "$name" >&2; return 1; }
  mv "$dir/$name" "$dest" || return 1
  # snapshot this skill's usage beside the archive
  jq --arg n "$name" '.[$n] // {}' <<<"$usage" >"$dest/.usage.json" 2>/dev/null || true
  usage=$(jq --arg n "$name" 'del(.[$n])' <<<"$usage")
  ps_save_usage "$(ps_usage_path "$root")" "$usage" || return 1
  printf 'archived %s\n' "$name"
}

ps_restore() {
  local name="$1"
  local pair root dir src usage snap
  pair=$(ps_require_name "$name") || return 1
  root=${pair%%	*}
  dir=${pair#*	}
  src="$dir/.archive/$name"
  [ -d "$src" ] || { printf 'skills.sh: no archived skill "%s"\n' "$name" >&2; return 1; }
  [ ! -e "$dir/$name" ] || { printf 'skills.sh: active skill "%s" already exists\n' "$name" >&2; return 1; }
  mv "$src" "$dir/$name" || return 1
  usage=$(ps_load_usage "$(ps_usage_path "$root")")
  if [ -f "$dir/$name/.usage.json" ]; then
    snap=$(jq -e . "$dir/$name/.usage.json" 2>/dev/null || printf '{}')
    rm -f "$dir/$name/.usage.json"
    usage=$(jq --arg n "$name" --argjson e "$snap" '.[$n]=($e + {state:"active"})' <<<"$usage")
  else
    usage=$(jq --arg n "$name" --arg t "$(ps_now)" \
      '.[$n] = ((.[$n] // {}) + {state:"active",origin:(.[$n].origin // "agent")})' <<<"$usage")
  fi
  ps_save_usage "$(ps_usage_path "$root")" "$usage" || return 1
  printf 'restored %s\n' "$name"
}

ps_set_pin() {
  local name="$1" val="$2"
  local pair root dir usage
  pair=$(ps_require_name "$name") || return 1
  root=${pair%%	*}
  dir=${pair#*	}
  [ -f "$dir/$name/SKILL.md" ] || { printf 'skills.sh: no skill "%s"\n' "$name" >&2; return 1; }
  usage=$(ps_load_usage "$(ps_usage_path "$root")")
  usage=$(jq --arg n "$name" --argjson p "$val" --arg t "$(ps_now)" '
    .[$n] = ((.[$n] // {origin:"agent",created_at:$t,use_count:0,patch_count:0,state:"active",first_seen_at:$t}) + {pinned:$p})
  ' <<<"$usage") || return 1
  ps_save_usage "$(ps_usage_path "$root")" "$usage" || return 1
  if [ "$val" = true ]; then printf 'pinned %s\n' "$name"; else printf 'unpinned %s\n' "$name"; fi
}

ps_adopt() {
  local name="$1"
  local pair root dir usage file origin created body desc
  pair=$(ps_require_name "$name") || return 1
  root=${pair%%	*}
  dir=${pair#*	}
  file="$dir/$name/SKILL.md"
  [ -f "$file" ] || { printf 'skills.sh: no skill "%s"\n' "$name" >&2; return 1; }
  desc=$(ps_skill_description "$file")
  [ -n "$desc" ] || desc="$name"
  usage=$(ps_load_usage "$(ps_usage_path "$root")")
  created=$(jq -r --arg n "$name" '.[$n].created_at // empty' <<<"$usage")
  if [ -z "$created" ] || [ "$created" = "null" ]; then created=$(ps_now); fi
  body=$(ps_strip_frontmatter <"$file")
  body=$(printf '%s\n' "$body" | sed '1{/^$/d;}')
  ps_render_skill "$name" "$desc" agent "$created" "$body" >"$file" || return 1
  usage=$(jq --arg n "$name" --arg t "$created" --arg nnow "$(ps_now)" '
    .[$n] = ((.[$n] // {use_count:0,patch_count:0,state:"active",pinned:false,first_seen_at:$nnow,created_at:$t}) + {origin:"agent"})
  ' <<<"$usage") || return 1
  ps_save_usage "$(ps_usage_path "$root")" "$usage" || return 1
  printf 'adopted %s\n' "$name"
}

ps_status() {
  local root dir usage
  root=$(ps_repo_root "${PS_CWD:-.}")
  [ -n "$root" ] || { printf 'skills.sh: not in a git repo\n' >&2; return 1; }
  dir=$(ps_skills_dir "$root")
  usage=$(ps_load_usage "$(ps_usage_path "$root")")
  jq -r --argjson u "$usage" '
    def n(s): [$u | to_entries[] | select(.value.state==s)] | length;
    "active\t\(n("active"))",
    "stale\t\(n("stale"))",
    "pinned\t\([$u | to_entries[] | select(.value.pinned==true) | .key] | join(","))",
    "lru\t\([$u | to_entries | sort_by(.value.last_used_at // "") | .[].key] | .[0:5] | join(","))"
  '
  local arch
  arch=$(find "$dir/.archive" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l | tr -d ' ')
  printf 'archived\t%s\n' "${arch:-0}"
}

# ps_record KIND NAME — kind is use|patch. Never fails the caller (prints nothing).
ps_record() {
  local kind="$1" name="$2"
  local root dir path usage now file origin
  root=$(ps_repo_root "${PS_CWD:-.}")
  [ -n "$root" ] || return 0
  dir=$(ps_skills_dir "$root")
  file="$dir/$name/SKILL.md"
  [ -f "$file" ] || return 0
  now=$(ps_now)
  origin=$(ps_skill_origin "$file")
  path=$(ps_usage_path "$root")
  usage=$(ps_load_usage "$path")
  usage=$(jq --arg n "$name" --arg k "$kind" --arg t "$now" --arg o "${origin:-}" '
    .[$n] = ((.[$n] // {
      origin: (if $o == "" then "unmanaged" else $o end),
      created_at: $t, use_count: 0, patch_count: 0,
      state: "active", pinned: false, first_seen_at: $t
    }) | . as $e | $e + (
      if $k == "use" then
        {use_count: ((.use_count // 0) + 1), last_used_at: $t, state: (if .state == "stale" then "active" else .state end)}
      else
        {patch_count: ((.patch_count // 0) + 1), last_patched_at: $t, state: (if .state == "stale" then "active" else .state end)}
      end
    ))
  ' <<<"$usage") || return 0
  ps_save_usage "$path" "$usage" || printf 'project-skills: failed to write %s\n' "$path" >&2
}

# Seed missing usage entries for on-disk agent-created skills. Prints updated JSON.
ps_seed_missing() {
  local dir="$1" usage="$2" now="$3" name origin created entry _seeded
  for name in $(ps_list_skill_names "$dir"); do
    origin=$(ps_skill_origin "$dir/$name/SKILL.md")
    [ "$origin" = "agent" ] || continue
    if jq -e --arg n "$name" 'has($n)' <<<"$usage" >/dev/null 2>&1; then
      continue
    fi
    created=$(ps_now)
    entry=$(ps_default_entry agent "$created" "$now")
    _seeded=$(jq --arg n "$name" --argjson e "$entry" '.[$n]=$e' <<<"$usage") || {
      printf 'project-skills: failed to seed usage for %s\n' "$name" >&2
      continue
    }
    usage=$_seeded
  done
  printf '%s' "$usage"
}

# 0 when elapsed time is at least DAYS full 86400-second periods.
ps_idle_at_least() {
  local iso="$1" days="$2" now_e then_e
  now_e=$(ps_now_epoch)
  then_e=$(ps_iso_to_epoch "$iso") || return 1
  [ $((now_e - then_e)) -ge $((days * 86400)) ]
}

# Display helper (ceil of elapsed days).
ps_idle_days() {
  local iso="$1" now_e then_e
  now_e=$(ps_now_epoch)
  then_e=$(ps_iso_to_epoch "$iso") || { printf '0'; return 0; }
  printf '%s' $(( (now_e - then_e + 86399) / 86400 ))
}

ps_curate() {
  local dry="${1:-0}"
  local root dir usage now name origin pinned state ts days dest
  root=$(ps_repo_root "${PS_CWD:-.}")
  [ -n "$root" ] || { printf 'skills.sh: not in a git repo\n' >&2; return 1; }
  dir=$(ps_skills_dir "$root")
  [ -d "$dir" ] || return 0
  ps_thresholds
  now=$(ps_now)
  usage=$(ps_load_usage "$(ps_usage_path "$root")")
  usage=$(ps_seed_missing "$dir" "$usage" "$now")
  if [ "$dry" != 1 ]; then
    ps_save_usage "$(ps_usage_path "$root")" "$usage" || {
      printf 'skills.sh: could not write usage.json\n' >&2
      return 0
    }
  fi
  for name in $(ps_list_skill_names "$dir"); do
    origin=$(jq -r --arg n "$name" '.[$n].origin // empty' <<<"$usage")
    [ -n "$origin" ] || origin=$(ps_skill_origin "$dir/$name/SKILL.md")
    [ "$origin" = "agent" ] || continue
    pinned=$(jq -r --arg n "$name" '.[$n].pinned // false' <<<"$usage")
    [ "$pinned" = "true" ] && continue
    ts=$(jq -r --arg n "$name" '.[$n].last_used_at // .[$n].first_seen_at // .[$n].created_at // empty' <<<"$usage")
    if [ -z "$ts" ] || [ "$ts" = "null" ]; then
      continue
    fi
    days=$(ps_idle_days "$ts")
    if ps_idle_at_least "$ts" "$PS_ARCHIVE"; then
      local uc
      uc=$(jq -r --arg n "$name" '.[$n].use_count // 0' <<<"$usage")
      # Used skills (use_count>0) archive at archiveAfterDays.
      # Never-used skills need the staleAfterDays grace floor first.
      if [ "$uc" = "0" ]; then
        ps_idle_at_least "$ts" "$PS_STALE" || continue
      fi
      if [ "$dry" = 1 ]; then
        printf 'would archive %s (%sd idle)\n' "$name" "$days"
        continue
      fi
      dest="$dir/.archive/$name"
      mkdir -p "$dir/.archive" || {
        printf 'skills.sh: cannot create archive dir\n' >&2
        continue
      }
      if [ -e "$dest" ]; then
        printf 'skills.sh: archive collision for %s — skipped\n' "$name" >&2
        continue
      fi
      if mv "$dir/$name" "$dest" 2>/dev/null; then
        jq --arg n "$name" '.[$n] // {}' <<<"$usage" >"$dest/.usage.json" 2>/dev/null || true
        usage=$(jq --arg n "$name" 'del(.[$n])' <<<"$usage")
        printf 'archived %s\n' "$name"
      else
        printf 'skills.sh: failed to archive %s — left in place\n' "$name" >&2
      fi
    elif ps_idle_at_least "$ts" "$PS_STALE"; then
      if [ "$dry" = 1 ]; then
        printf 'would stale %s (%sd idle)\n' "$name" "$days"
        continue
      fi
      usage=$(jq --arg n "$name" '.[$n].state = "stale"' <<<"$usage")
    fi
  done
  if [ "$dry" != 1 ]; then
    ps_save_usage "$(ps_usage_path "$root")" "$usage" \
      || printf 'project-skills: failed to write usage.json\n' >&2
  fi
}
