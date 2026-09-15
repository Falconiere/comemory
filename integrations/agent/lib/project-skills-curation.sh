#!/usr/bin/env bash
# shellcheck shell=bash
# Project-skill use accounting and stale/archive lifecycle curation.

ps_record() {
  local kind="$1" name="$2" root dir path usage now file origin
  root=$(ps_repo_root "${PS_CWD:-.}"); [ -n "$root" ] || return 0; dir=$(ps_skills_dir "$root"); file="$dir/$name/SKILL.md"; [ -f "$file" ] || return 0
  now=$(ps_now); origin=$(ps_skill_origin "$file"); path=$(ps_usage_path "$root"); usage=$(ps_load_usage "$path")
  usage=$(jq --arg n "$name" --arg k "$kind" --arg t "$now" --arg o "${origin:-}" '
    .[$n] = ((.[$n] // {origin:(if $o == "" then "unmanaged" else $o end),created_at:$t,use_count:0,patch_count:0,state:"active",pinned:false,first_seen_at:$t}) | . as $e | $e + (if $k == "use" then {use_count:((.use_count // 0)+1),last_used_at:$t,state:(if .state == "stale" then "active" else .state end)} else {patch_count:((.patch_count // 0)+1),last_patched_at:$t,state:(if .state == "stale" then "active" else .state end)} end))
  ' <<<"$usage") || return 0
  ps_save_usage "$path" "$usage" || printf 'project-skills: failed to write %s\n' "$path" >&2
}
ps_seed_missing() {
  local dir="$1" usage="$2" now="$3" name origin created entry seeded
  for name in $(ps_list_skill_names "$dir"); do
    origin=$(ps_skill_origin "$dir/$name/SKILL.md"); [ "$origin" = agent ] || continue
    jq -e --arg n "$name" 'has($n)' <<<"$usage" >/dev/null 2>&1 && continue
    created=$(ps_now); entry=$(ps_default_entry agent "$created" "$now")
    seeded=$(jq --arg n "$name" --argjson e "$entry" '.[$n]=$e' <<<"$usage") || { printf 'project-skills: failed to seed usage for %s\n' "$name" >&2; continue; }
    usage=$seeded
  done
  printf '%s' "$usage"
}
ps_idle_at_least() {
  local iso="$1" days="$2" now_e then_e
  now_e=$(ps_now_epoch); then_e=$(ps_iso_to_epoch "$iso") || return 1
  [ $((now_e - then_e)) -ge $((days * 86400)) ]
}
ps_idle_days() {
  local iso="$1" now_e then_e
  now_e=$(ps_now_epoch); then_e=$(ps_iso_to_epoch "$iso") || { printf '0'; return 0; }
  printf '%s' $(( (now_e - then_e + 86399) / 86400 ))
}
ps_curate() {
  local dry="${1:-0}" root dir usage now name origin pinned ts days dest
  root=$(ps_repo_root "${PS_CWD:-.}"); [ -n "$root" ] || { printf 'skills.sh: not in a git repo\n' >&2; return 1; }; dir=$(ps_skills_dir "$root"); [ -d "$dir" ] || return 0
  ps_thresholds; now=$(ps_now); usage=$(ps_load_usage "$(ps_usage_path "$root")"); usage=$(ps_seed_missing "$dir" "$usage" "$now")
  if [ "$dry" != 1 ]; then ps_save_usage "$(ps_usage_path "$root")" "$usage" || { printf 'skills.sh: could not write usage.json\n' >&2; return 0; }; fi
  for name in $(ps_list_skill_names "$dir"); do
    origin=$(jq -r --arg n "$name" '.[$n].origin // empty' <<<"$usage"); [ -n "$origin" ] || origin=$(ps_skill_origin "$dir/$name/SKILL.md"); [ "$origin" = agent ] || continue
    pinned=$(jq -r --arg n "$name" '.[$n].pinned // false' <<<"$usage"); [ "$pinned" != true ] || continue
    ts=$(jq -r --arg n "$name" '.[$n].last_used_at // .[$n].first_seen_at // .[$n].created_at // empty' <<<"$usage"); [ -n "$ts" ] && [ "$ts" != null ] || continue; days=$(ps_idle_days "$ts")
    if ps_idle_at_least "$ts" "$PS_ARCHIVE"; then
      local use_count
      use_count=$(jq -r --arg n "$name" '.[$n].use_count // 0' <<<"$usage"); [ "$use_count" != 0 ] || ps_idle_at_least "$ts" "$PS_STALE" || continue
      [ "$dry" != 1 ] || { printf 'would archive %s (%sd idle)\n' "$name" "$days"; continue; }
      dest="$dir/.archive/$name"; mkdir -p "$dir/.archive" || { printf 'skills.sh: cannot create archive dir\n' >&2; continue; }
      [ ! -e "$dest" ] || { printf 'skills.sh: archive collision for %s — skipped\n' "$name" >&2; continue; }
      if mv "$dir/$name" "$dest" 2>/dev/null; then jq --arg n "$name" '.[$n] // {}' <<<"$usage" >"$dest/.usage.json" 2>/dev/null || true; usage=$(jq --arg n "$name" 'del(.[$n])' <<<"$usage"); printf 'archived %s\n' "$name"; else printf 'skills.sh: failed to archive %s — left in place\n' "$name" >&2; fi
    elif ps_idle_at_least "$ts" "$PS_STALE"; then
      [ "$dry" != 1 ] || { printf 'would stale %s (%sd idle)\n' "$name" "$days"; continue; }
      usage=$(jq --arg n "$name" '.[$n].state = "stale"' <<<"$usage")
    fi
  done
  [ "$dry" = 1 ] || ps_save_usage "$(ps_usage_path "$root")" "$usage" || printf 'project-skills: failed to write usage.json\n' >&2
}
