#!/usr/bin/env bash
# Real native host installers + real CLI/Git/worktree memory round trip.
set -euo pipefail
ROOT="$(cd "${BASH_SOURCE%/*}/.." && pwd)"
BIN="${COMEMORY_BIN:-$ROOT/target/debug/comemory}"
[ -x "$BIN" ] || { printf 'Missing executable comemory binary: %s\n' "$BIN" >&2; exit 1; }
TASK=$(mktemp -d "${TMPDIR:-/tmp}/comemory-test.XXXXXX")
trap 'rm -rf "$TASK"' EXIT
mkdir -p "$TASK/bin"
ln -s "$BIN" "$TASK/bin/comemory"
export PATH="$TASK/bin:$PATH"
export COMEMORY_DATA_DIR="$TASK/data with spaces"
unset TOOLU_CONFIG_DIR TOOLU_HOST_OVERRIDE PLUGIN_ROOT CLAUDE_PLUGIN_ROOT

git init -q "$TASK/repository"
git -C "$TASK/repository" -c user.name=Comemory -c user.email=test@example.invalid commit -qm initial --allow-empty
git -C "$TASK/repository" worktree add -q -b sibling "$TASK/worktree"
for host in claude codex; do
  command -v "$host" >/dev/null
  cfg="$TASK/$host config"
  "$BIN" install "$host" --config-dir "$cfg" --json > "$TASK/install-$host.json"
  jq -e '.installed and .plugin == "comemory@comemory"' "$TASK/install-$host.json" >/dev/null
  plugin="$(jq -r .bundle "$TASK/install-$host.json")/plugins/comemory"
  # Host schema/install is exercised above; execute its shipped hook with real
  # payload and native host environment to verify wrapper publication.
  if [ "$host" = codex ]; then
    export CODEX_HOME="$cfg" PLUGIN_ROOT="$plugin"
    unset CLAUDE_CONFIG_DIR
  else
    export CLAUDE_CONFIG_DIR="$cfg"
    unset CODEX_HOME PLUGIN_ROOT
  fi
  export CLAUDE_PLUGIN_ROOT="$plugin"
  # Register a real older-version local bundle, then exercise upgrade in place.
  marketplace=$(jq -r .marketplace "$TASK/install-$host.json")
  python3 - "$marketplace" "$plugin" <<'PYUPGRADE'
import json, pathlib, shutil, sys
marketplace, current = map(pathlib.Path, sys.argv[1:])
old = marketplace / "0.0.1/plugins/comemory"
if not old.exists():
    shutil.copytree(current, old)
for manifest in old.glob(".*-plugin/plugin.json"):
    value = json.loads(manifest.read_text())
    value["version"] = "0.0.1"
    manifest.write_text(json.dumps(value))
for relative in [".claude-plugin/marketplace.json", ".agents/plugins/marketplace.json"]:
    catalog = marketplace / relative
    value = json.loads(catalog.read_text())
    source = value["plugins"][0]["source"]
    if isinstance(source, dict):
        source["path"] = "./0.0.1/plugins/comemory"
    else:
        value["plugins"][0]["source"] = "./0.0.1/plugins/comemory"
    catalog.write_text(json.dumps(value))
PYUPGRADE
  if [ "$host" = claude ]; then remove=uninstall; add=install; else remove=remove; add=add; fi
  "$host" plugin "$remove" comemory@comemory >/dev/null
  "$host" plugin marketplace add "$marketplace" >/dev/null
  "$host" plugin "$add" comemory@comemory >/dev/null
  "$BIN" install "$host" --config-dir "$cfg" --json >/dev/null
  "$host" plugin list --json > "$TASK/list-$host.json"
  jq -e --arg version "$($BIN --version | awk '{print $2}')" '.. | objects | select(.version? == $version)' "$TASK/list-$host.json" >/dev/null
  jq -nc --arg cwd "$TASK/repository" '{cwd:$cwd,session_id:"migration-integration",source:"startup",hook_event_name:"SessionStart"}' \
    | bash -c "$(jq -r '.hooks.SessionStart[0].hooks[0].command' "$plugin/hooks/hooks.json")" > "$TASK/start-$host.json"
  jq -e '.hookSpecificOutput.additionalContext | contains("repo-scoped recall")' "$TASK/start-$host.json" >/dev/null
  wrapper="$cfg/comemory/comemory.sh"
  badge="$plugin/hooks/comemory-status.sh"
  (cd "$TASK/repository" && "$wrapper" save "Worktree correction $host" 'Use the canonical repository scope; a worktree name splits retrieval. Verified by saving in the primary checkout and retrieving from its sibling.' --kind convention --json) > "$TASK/saved-$host.json"
  (cd "$TASK/worktree" && "$wrapper" search "Worktree correction $host" --json) > "$TASK/recalled-$host.json"
  jq -e --arg host "$host" 'tostring | contains("Worktree correction " + $host)' "$TASK/recalled-$host.json" >/dev/null
  jq -nc --arg cwd "$TASK/worktree" '{cwd:$cwd,tool_name:"Bash",tool_input:{command:"comemory save lesson"}}' \
    | "$plugin/hooks/scope.sh" > "$TASK/scope-$host.json"
  jq -e '.hookSpecificOutput.permissionDecision == "deny"' "$TASK/scope-$host.json" >/dev/null
  for invocation in 'env comemory search lesson' 'command comemory save lesson' 'env -u TEST_SCOPE command comemory search lesson'; do
    jq -nc --arg cwd "$TASK/worktree" --arg command "$invocation" '{cwd:$cwd,tool_name:"Bash",tool_input:{command:$command}}' \
      | "$plugin/hooks/scope.sh" | jq -e '.hookSpecificOutput.permissionDecision == "deny"' >/dev/null
  done
  # Concurrent badge refreshes publish valid JSON and clean their unique temps.
  badge_repo="$TASK/badge $host \"quoted\""
  mkdir -p "$badge_repo"
  git init -q "$badge_repo"
  (cd "$badge_repo" && "$wrapper" save "Badge lesson $host" "Verified badge fixture" --json) >/dev/null
  badge_payload=$(jq -nc --arg cwd "$badge_repo" '{cwd:$cwd}')
  printf '%s' "$badge_payload" | "$badge" &
  badge_pid=$!
  printf '%s' "$badge_payload" | "$badge"
  wait "$badge_pid"
  jq -e --arg repo "${badge_repo##*/}" '.repo == $repo and .count == 1' "$cfg/comemory-status/${badge_repo##*/}.json" >/dev/null
  leftover=$(find "$cfg/comemory-status" -name '.count.*' -print)
  [ -z "$leftover" ]
  printf blocked > "$TASK/blocked-config"
  printf '%s' "$badge_payload" | TOOLU_CONFIG_DIR="$TASK/blocked-config" "$badge"
  # Exact argv handling, explicit scope, real CLI failure, and disable controls.
  body=$'User correction: "quoted" title\nEvidence: a real Git worktree shares its parent repository.'
  (cd "$TASK/worktree" && "$wrapper" save "Quoted lesson $host" "$body" --repo overridden --json) >/dev/null
  (cd "$TASK/repository" && "$wrapper" search "Quoted lesson $host" --repo overridden --json) \
    | jq -e 'tostring | contains("quoted")' >/dev/null
  if (cd "$TASK/repository" && "$wrapper" save invalid body --kind invalid) >"$TASK/failure-$host" 2>&1; then
    printf 'Expected invalid memory kind to fail\n' >&2; exit 1
  fi
  python3 - "$TASK/failure-$host" <<'PYERROR'
import pathlib, sys
error = pathlib.Path(sys.argv[1]).read_text()
assert "invalid value 'invalid' for '--kind <KIND>'" in error, error
PYERROR
  printf '%s\n' '{"projectSkills":{"enabled":false}}' > "$cfg/comemory.json"
  jq -nc --arg cwd "$TASK/repository" '{cwd:$cwd}' | "$plugin/hooks/session-start.sh" \
    | jq -e '.hookSpecificOutput.additionalContext' >/dev/null
  printf '%s\n' '{"skills":{"comemory":false}}' > "$cfg/comemory.json"
  disabled=$(jq -nc --arg cwd "$TASK/repository" '{cwd:$cwd}' | "$plugin/hooks/session-start.sh")
  [ -z "$disabled" ]
  rm "$cfg/comemory.json"
  printf 'agent-install: %s native install/upgrade, SessionStart, worktree recall, scope denial passed\n' "$host"
done
