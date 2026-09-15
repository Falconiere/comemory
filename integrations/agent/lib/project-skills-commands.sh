#!/usr/bin/env bash
# shellcheck shell=bash
# Project-skill create, listing, archive, restore, pin, adoption, and status.

ps_create() {
  local name="$1" desc="$2" src="$3" root dir dest body created now usage path entry
  root=$(ps_repo_root "${PS_CWD:-.}")
  [ -n "$root" ] || { printf 'skills.sh: not in a git repo\n' >&2; return 1; }
  ps_valid_name "$name" || { printf 'skills.sh: invalid name "%s" (1–64 chars, [a-z][a-z0-9-]*)\n' "$name" >&2; return 1; }
  [ "$(ps_word_count "$desc")" -le 30 ] || { printf 'skills.sh: description must be ≤ 30 words\n' >&2; return 1; }
  [ -n "$desc" ] || { printf 'skills.sh: --description is required\n' >&2; return 1; }
  if [ -n "$src" ]; then [ -f "$src" ] || { printf 'skills.sh: --file %s not found\n' "$src" >&2; return 1; }; body=$(ps_strip_frontmatter <"$src"); else body=$(ps_strip_frontmatter); fi
  body=$(printf '%s\n' "$body" | sed '1{/^$/d;}')
  ps_has_required_headings "$body" || { printf 'skills.sh: body must contain ## When to Use, ## Procedure, ## Pitfalls, ## Verification\n' >&2; return 1; }
  dir=$(ps_skills_dir "$root"); dest="$dir/$name/SKILL.md"
  [ ! -e "$dir/$name" ] || { printf 'skills.sh: skill "%s" already exists\n' "$name" >&2; return 1; }
  ps_ensure_tree "$dir" || return 1; mkdir -p "$dir/$name" || return 1
  created=$(ps_now); now=$created; ps_render_skill "$name" "$desc" agent "$created" "$body" >"$dest" || return 1
  path=$(ps_usage_path "$root"); usage=$(ps_load_usage "$path"); entry=$(ps_default_entry agent "$created" "$now")
  usage=$(jq --arg n "$name" --argjson e "$entry" '.[$n]=$e' <<<"$usage") || return 1
  ps_save_usage "$path" "$usage" || return 1; printf 'created %s\n' "$dest"
}
ps_list() {
  local json="$1" root dir usage name origin state
  root=$(ps_repo_root "${PS_CWD:-.}"); [ -n "$root" ] || { printf 'skills.sh: not in a git repo\n' >&2; return 1; }
  dir=$(ps_skills_dir "$root"); usage=$(ps_load_usage "$(ps_usage_path "$root")")
  if [ "$json" = 1 ]; then jq -n --argjson u "$usage" '$u | with_entries(select(.value.state == "active" or .value.state == "stale"))'; return 0; fi
  for name in $(ps_list_skill_names "$dir"); do origin=$(ps_skill_origin "$dir/$name/SKILL.md"); state=$(jq -r --arg n "$name" '.[$n].state // "active"' <<<"$usage"); printf '%s\t%s\t%s\n' "$name" "${origin:-unmanaged}" "$state"; done
}
ps_index() {
  local root dir usage cap name desc ts ranked tmp
  root=$(ps_repo_root "${PS_CWD:-.}"); [ -n "$root" ] || return 0; dir=$(ps_skills_dir "$root"); [ -d "$dir" ] || return 0
  ps_thresholds; cap=$PS_INDEX_CAP; usage=$(ps_load_usage "$(ps_usage_path "$root")"); tmp=$(mktemp) || return 0
  for name in $(ps_list_skill_names "$dir"); do
    ts=$(jq -r --arg n "$name" '.[$n].last_used_at // .[$n].created_at // .[$n].first_seen_at // ""' <<<"$usage" 2>/dev/null || true); [ "$ts" = null ] && ts=""; printf '%s\t%s\n' "$ts" "$name"
  done | sort -r >"$tmp"
  ranked=$(awk -F '\t' 'NF>=2 {print $2}' "$tmp" | head -n "$cap" || true); rm -f "$tmp"; [ -n "$ranked" ] || return 0
  while IFS= read -r name; do desc=$(ps_skill_description "$dir/$name/SKILL.md"); [ -n "$desc" ] && printf -- '- %s: %s\n' "$name" "$desc"; done <<EOF
$ranked
EOF
}
ps_require_name() {
  local name="$1" root dir
  ps_valid_name "$name" || { printf 'skills.sh: invalid name "%s"\n' "$name" >&2; return 1; }
  root=$(ps_repo_root "${PS_CWD:-.}"); [ -n "$root" ] || { printf 'skills.sh: not in a git repo\n' >&2; return 1; }
  dir=$(ps_skills_dir "$root"); printf '%s\t%s' "$root" "$dir"
}
ps_archive() {
  local name="$1" dry="${2:-}" pair root dir usage origin pinned dest
  pair=$(ps_require_name "$name") || return 1; root=${pair%%$'\t'*}; dir=${pair#*$'\t'}
  [ -d "$dir/$name" ] || { printf 'skills.sh: no skill "%s"\n' "$name" >&2; return 1; }
  usage=$(ps_load_usage "$(ps_usage_path "$root")"); origin=$(jq -r --arg n "$name" '.[$n].origin // empty' <<<"$usage"); [ -n "$origin" ] || origin=$(ps_skill_origin "$dir/$name/SKILL.md")
  [ "$origin" = agent ] || { printf 'skills.sh: "%s" is unmanaged (adopt first)\n' "$name" >&2; return 1; }
  pinned=$(jq -r --arg n "$name" '.[$n].pinned // false' <<<"$usage"); [ "$pinned" != true ] || { printf 'skills.sh: "%s" is pinned\n' "$name" >&2; return 1; }
  dest="$(ps_skills_dir "$root")/.archive/$name"; [ "$dry" != 1 ] || { printf 'would archive %s -> %s\n' "$name" "$dest"; return 0; }
  mkdir -p "$(dirname "$dest")" || return 1; [ ! -e "$dest" ] || { printf 'skills.sh: archive already has "%s"\n' "$name" >&2; return 1; }
  mv "$dir/$name" "$dest" || return 1; jq --arg n "$name" '.[$n] // {}' <<<"$usage" >"$dest/.usage.json" 2>/dev/null || true
  usage=$(jq --arg n "$name" 'del(.[$n])' <<<"$usage"); ps_save_usage "$(ps_usage_path "$root")" "$usage" || return 1; printf 'archived %s\n' "$name"
}
ps_restore() {
  local name="$1" pair root dir src usage snap
  pair=$(ps_require_name "$name") || return 1; root=${pair%%$'\t'*}; dir=${pair#*$'\t'}; src="$dir/.archive/$name"
  [ -d "$src" ] || { printf 'skills.sh: no archived skill "%s"\n' "$name" >&2; return 1; }; [ ! -e "$dir/$name" ] || { printf 'skills.sh: active skill "%s" already exists\n' "$name" >&2; return 1; }
  mv "$src" "$dir/$name" || return 1; usage=$(ps_load_usage "$(ps_usage_path "$root")")
  if [ -f "$dir/$name/.usage.json" ]; then snap=$(jq -e . "$dir/$name/.usage.json" 2>/dev/null || printf '{}'); rm -f "$dir/$name/.usage.json"; usage=$(jq --arg n "$name" --argjson e "$snap" '.[$n]=($e + {state:"active"})' <<<"$usage");
  else usage=$(jq --arg n "$name" --arg t "$(ps_now)" '.[$n] = ((.[$n] // {}) + {state:"active",origin:(.[$n].origin // "agent")})' <<<"$usage"); fi
  ps_save_usage "$(ps_usage_path "$root")" "$usage" || return 1; printf 'restored %s\n' "$name"
}
ps_set_pin() {
  local name="$1" val="$2" pair root dir usage
  pair=$(ps_require_name "$name") || return 1; root=${pair%%$'\t'*}; dir=${pair#*$'\t'}; [ -f "$dir/$name/SKILL.md" ] || { printf 'skills.sh: no skill "%s"\n' "$name" >&2; return 1; }
  usage=$(ps_load_usage "$(ps_usage_path "$root")"); usage=$(jq --arg n "$name" --argjson p "$val" --arg t "$(ps_now)" '.[$n] = ((.[$n] // {origin:"agent",created_at:$t,use_count:0,patch_count:0,state:"active",first_seen_at:$t}) + {pinned:$p})' <<<"$usage") || return 1
  ps_save_usage "$(ps_usage_path "$root")" "$usage" || return 1; if [ "$val" = true ]; then printf 'pinned %s\n' "$name"; else printf 'unpinned %s\n' "$name"; fi
}
ps_adopt() {
  local name="$1" pair root dir usage file created body desc
  pair=$(ps_require_name "$name") || return 1; root=${pair%%$'\t'*}; dir=${pair#*$'\t'}; file="$dir/$name/SKILL.md"
  [ -f "$file" ] || { printf 'skills.sh: no skill "%s"\n' "$name" >&2; return 1; }; desc=$(ps_skill_description "$file"); [ -n "$desc" ] || desc="$name"; usage=$(ps_load_usage "$(ps_usage_path "$root")")
  created=$(jq -r --arg n "$name" '.[$n].created_at // empty' <<<"$usage"); [ -n "$created" ] && [ "$created" != null ] || created=$(ps_now)
  body=$(ps_strip_frontmatter <"$file"); body=$(printf '%s\n' "$body" | sed '1{/^$/d;}'); ps_render_skill "$name" "$desc" agent "$created" "$body" >"$file" || return 1
  usage=$(jq --arg n "$name" --arg t "$created" --arg nnow "$(ps_now)" '.[$n] = ((.[$n] // {use_count:0,patch_count:0,state:"active",pinned:false,first_seen_at:$nnow,created_at:$t}) + {origin:"agent"})' <<<"$usage") || return 1
  ps_save_usage "$(ps_usage_path "$root")" "$usage" || return 1; printf 'adopted %s\n' "$name"
}
ps_status() {
  local root dir usage arch
  root=$(ps_repo_root "${PS_CWD:-.}"); [ -n "$root" ] || { printf 'skills.sh: not in a git repo\n' >&2; return 1; }; dir=$(ps_skills_dir "$root"); usage=$(ps_load_usage "$(ps_usage_path "$root")")
  jq -r --argjson u "$usage" 'def n(s): [$u | to_entries[] | select(.value.state==s)] | length; "active\t\(n("active"))", "stale\t\(n("stale"))", "pinned\t\([$u | to_entries[] | select(.value.pinned==true) | .key] | join(","))", "lru\t\([$u | to_entries | sort_by(.value.last_used_at // "") | .[].key] | .[0:5] | join(","))"'
  arch=$(find "$dir/.archive" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l | tr -d ' '); printf 'archived\t%s\n' "${arch:-0}"
}
