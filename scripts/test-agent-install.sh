#!/usr/bin/env bash
# Real native host installers + real CLI/Git/worktree memory round trip.
set -euo pipefail
ROOT="$(cd "${BASH_SOURCE%/*}/.." && pwd)"
BIN="${COMEMORY_BIN:-$ROOT/target/debug/comemory}"
[ -x "$BIN" ] || { printf 'Missing executable comemory binary: %s\n' "$BIN" >&2; exit 1; }
TASK=$(mktemp -d "${TMPDIR:-/tmp}/comemory-test.XXXXXX")
# The Stop hook's detached maintenance may still be writing under $TASK for a
# few seconds; retry the sweep once so a late child rarely leaves a temp dir,
# and name the leftover on stderr when the retry fails too.
trap 'rm -rf "$TASK" 2>/dev/null || { sleep 3; rm -rf "$TASK" 2>/dev/null || printf "warning: temp dir left behind: %s\n" "$TASK" >&2; }' EXIT
mkdir -p "$TASK/bin"
ln -s "$BIN" "$TASK/bin/comemory"
export PATH="$TASK/bin:$PATH"
export COMEMORY_DATA_DIR="$TASK/data with spaces"
unset TOOLU_CONFIG_DIR TOOLU_HOST_OVERRIDE PLUGIN_ROOT CLAUDE_PLUGIN_ROOT

test_source_hook_regressions() {
  local source_agent="$ROOT/integrations/agent"
  local source_cfg="$TASK/source config"
  local source_repo="$TASK/source-hooks-repo"
  local source_key disabled_payload disabled_out
  local start_a start_b stop_b stop_b_out stop_b_out2 empty_stop empty_stop_out
  local resume_start old_start old_blocked old_advised eight_days_ago
  local race_start race_stop race_pid nonempty_race push_out
  local many_start many_stop many_out first_three fourth_id fifth_id

  mkdir -p "$source_cfg" "$source_repo"
  git init -q "$source_repo"
  source_key=$(basename "$source_repo")

  # Disabling comemory suppresses the empty-repository bootstrap hint as well
  # as the standing SessionStart reminder. Badge refresh is allowed to remain.
  printf '%s\n' '{"skills":{"comemory":false}}' > "$source_cfg/comemory.json"
  disabled_payload=$(jq -nc --arg cwd "$source_repo" '{cwd:$cwd,hook_event_name:"SessionStart"}')
  disabled_out=$(printf '%s' "$disabled_payload" \
    | TOOLU_CONFIG_DIR="$source_cfg" TOOLU_HOST_OVERRIDE=claude \
      "$source_agent/hooks/comemory-status.sh")
  [ -z "$disabled_out" ] || {
    printf 'disabled comemory emitted a bootstrap hint: %s\n' "$disabled_out" >&2
    exit 1
  }
  rm -f "$source_cfg/comemory.json"

  # The recall-status window is repository-wide. A starts a recall after B's
  # marker, so B's Stop can see it but must never claim ownership or block B.
  (cd "$source_repo" && "$BIN" save \
    "Concurrent recall fixture: a repository-wide recall cannot be attributed to one host session." \
    --repo "$source_key" --json) >/dev/null
  start_a=$(jq -nc --arg cwd "$source_repo" \
    '{cwd:$cwd,session_id:"source-agent-a",source:"startup",hook_event_name:"SessionStart"}')
  start_b=$(jq -nc --arg cwd "$source_repo" \
    '{cwd:$cwd,session_id:"source-agent-b",source:"startup",hook_event_name:"SessionStart"}')
  printf '%s' "$start_a" | TOOLU_CONFIG_DIR="$source_cfg" TOOLU_HOST_OVERRIDE=claude \
    "$source_agent/hooks/session-start.sh" >/dev/null
  printf '%s' "$start_b" | TOOLU_CONFIG_DIR="$source_cfg" TOOLU_HOST_OVERRIDE=claude \
    "$source_agent/hooks/session-start.sh" >/dev/null
  (cd "$source_repo" && "$BIN" search "Concurrent recall fixture" \
    --repo "$source_key" --json) >/dev/null
  stop_b=$(jq -nc --arg cwd "$source_repo" \
    '{cwd:$cwd,session_id:"source-agent-b",stop_hook_active:false,hook_event_name:"Stop"}')
  stop_b_out=$(printf '%s' "$stop_b" \
    | TOOLU_CONFIG_DIR="$source_cfg" TOOLU_HOST_OVERRIDE=claude \
      "$source_agent/hooks/memory-lifecycle.sh")
  jq -e 'has("decision") | not' <<<"$stop_b_out" >/dev/null
  jq -e '.systemMessage | contains("shared repository activity")' <<<"$stop_b_out" >/dev/null
  [ -d "$source_cfg/comemory/maintain-$(date -u +%Y%m%d)" ] || {
    printf 'Stop advisory skipped the independent daily maintenance latch\n' >&2
    exit 1
  }
  stop_b_out2=$(printf '%s' "$stop_b" \
    | TOOLU_CONFIG_DIR="$source_cfg" TOOLU_HOST_OVERRIDE=claude \
      "$source_agent/hooks/memory-lifecycle.sh")
  [ -z "$stop_b_out2" ]

  # A long shared window keeps its full pending count but bounds the example
  # query IDs. It also makes clear that another session's work is optional.
  many_start=$(jq -nc --arg cwd "$source_repo" \
    '{cwd:$cwd,session_id:"source-many",source:"startup",hook_event_name:"SessionStart"}')
  printf '%s' "$many_start" | TOOLU_CONFIG_DIR="$source_cfg" TOOLU_HOST_OVERRIDE=claude \
    "$source_agent/hooks/session-start.sh" >/dev/null
  : > "$TASK/many-searches.jsonl"
  for suffix in one two three four five; do
    (cd "$source_repo" && "$BIN" search "Concurrent recall fixture $suffix" \
      --repo "$source_key" --json) >> "$TASK/many-searches.jsonl"
  done
  jq -s '[.[].query_id]' "$TASK/many-searches.jsonl" > "$TASK/many-query-ids.json"
  first_three=$(jq -r '.[0:3] | join(", ")' "$TASK/many-query-ids.json")
  fourth_id=$(jq -r '.[3]' "$TASK/many-query-ids.json")
  fifth_id=$(jq -r '.[4]' "$TASK/many-query-ids.json")
  many_stop=$(jq -nc --arg cwd "$source_repo" \
    '{cwd:$cwd,session_id:"source-many",stop_hook_active:false,hook_event_name:"Stop"}')
  many_out=$(printf '%s' "$many_stop" \
    | TOOLU_CONFIG_DIR="$source_cfg" TOOLU_HOST_OVERRIDE=claude \
      "$source_agent/hooks/memory-lifecycle.sh")
  jq -e --arg ids "$first_three" --arg fourth "$fourth_id" --arg fifth "$fifth_id" '
    .systemMessage
    | contains("5 unjudged recall(s)")
      and contains("e.g. query IDs: " + $ids)
      and (contains($fourth) | not)
      and (contains($fifth) | not)
      and contains("If useful, inspect recall_status")
      and contains("No action is required for work from another session")' <<<"$many_out" >/dev/null

  # A tracked lookup with no returned ids cannot receive an honest per-memory
  # verdict, so Stop must omit it instead of requesting fabricated feedback.
  start_a=$(jq -nc --arg cwd "$source_repo" \
    '{cwd:$cwd,session_id:"source-empty-results",source:"startup",hook_event_name:"SessionStart"}')
  printf '%s' "$start_a" | TOOLU_CONFIG_DIR="$source_cfg" TOOLU_HOST_OVERRIDE=claude \
    "$source_agent/hooks/session-start.sh" >/dev/null
  (cd "$source_repo" && "$BIN" search "no-match-9f826bb50d1b" \
    --repo "$source_key" --json) >/dev/null
  empty_stop=$(jq -nc --arg cwd "$source_repo" \
    '{cwd:$cwd,session_id:"source-empty-results",stop_hook_active:false,hook_event_name:"Stop"}')
  empty_stop_out=$(printf '%s' "$empty_stop" \
    | TOOLU_CONFIG_DIR="$source_cfg" TOOLU_HOST_OVERRIDE=claude \
      "$source_agent/hooks/memory-lifecycle.sh")
  [ -z "$empty_stop_out" ] || {
    printf 'empty returned_ids produced a Stop advisory: %s\n' "$empty_stop_out" >&2
    exit 1
  }

  # Resume keeps its own write-once marker even when old, while the same sweep
  # removes stale start markers and both legacy/new advisory latch markers.
  resume_start=$(jq -nc --arg cwd "$source_repo" \
    '{cwd:$cwd,session_id:"source-resume",source:"startup",hook_event_name:"SessionStart"}')
  printf '%s' "$resume_start" | TOOLU_CONFIG_DIR="$source_cfg" TOOLU_HOST_OVERRIDE=claude \
    "$source_agent/hooks/session-start.sh" >/dev/null
  old_start="$source_cfg/comemory/session-old.start"
  old_blocked="$source_cfg/comemory/session-old.blocked"
  old_advised="$source_cfg/comemory/session-old.advised"
  : > "$old_start"
  : > "$old_blocked"
  : > "$old_advised"
  eight_days_ago=$(date -v-8d +%Y%m%d%H%M 2>/dev/null || date -d '8 days ago' +%Y%m%d%H%M)
  touch -t "$eight_days_ago" "$old_start" "$old_blocked" "$old_advised" \
    "$source_cfg/comemory/session-source-resume.start"
  printf '%s' "$resume_start" | TOOLU_CONFIG_DIR="$source_cfg" TOOLU_HOST_OVERRIDE=claude \
    "$source_agent/hooks/session-start.sh" >/dev/null
  [ -f "$source_cfg/comemory/session-source-resume.start" ]
  [ ! -e "$old_start" ] && [ ! -e "$old_blocked" ] && [ ! -e "$old_advised" ]

  # Duplicate Stop invocations for one session race on the advisory latch;
  # atomic creation permits exactly one systemMessage.
  race_start=$(jq -nc --arg cwd "$source_repo" \
    '{cwd:$cwd,session_id:"source-race",source:"startup",hook_event_name:"SessionStart"}')
  printf '%s' "$race_start" | TOOLU_CONFIG_DIR="$source_cfg" TOOLU_HOST_OVERRIDE=claude \
    "$source_agent/hooks/session-start.sh" >/dev/null
  (cd "$source_repo" && "$BIN" search "Concurrent recall fixture" \
    --repo "$source_key" --json) >/dev/null
  race_stop=$(jq -nc --arg cwd "$source_repo" \
    '{cwd:$cwd,session_id:"source-race",stop_hook_active:false,hook_event_name:"Stop"}')
  printf '%s' "$race_stop" | TOOLU_CONFIG_DIR="$source_cfg" TOOLU_HOST_OVERRIDE=claude \
    "$source_agent/hooks/memory-lifecycle.sh" > "$TASK/race-stop-1.json" &
  race_pid=$!
  printf '%s' "$race_stop" | TOOLU_CONFIG_DIR="$source_cfg" TOOLU_HOST_OVERRIDE=claude \
    "$source_agent/hooks/memory-lifecycle.sh" > "$TASK/race-stop-2.json"
  wait "$race_pid"
  nonempty_race=0
  [ ! -s "$TASK/race-stop-1.json" ] || nonempty_race=$((nonempty_race + 1))
  [ ! -s "$TASK/race-stop-2.json" ] || nonempty_race=$((nonempty_race + 1))
  [ "$nonempty_race" -eq 1 ]

  # An injected hint comes from an untracked lookup and has no query_id. It
  # directs selective show without asking for a redundant find or a verdict
  # that cannot be attached to this hint.
  push_out=$(jq -nc --arg cwd "$source_repo" \
    --arg prompt "where is the concurrent recall fixture documented" \
    '{cwd:$cwd,hook_event_name:"UserPromptSubmit",prompt:$prompt}' \
    | TOOLU_CONFIG_DIR="$source_cfg" TOOLU_HOST_OVERRIDE=claude \
      "$source_agent/hooks/memory-lifecycle.sh")
  jq -e '.hookSpecificOutput.additionalContext
    | contains("selectively show")
      and contains("Feedback applies only to a separate tracked recall with a query_id")
      and (contains("then feedback") | not)
      and (contains("Call find") | not)' <<<"$push_out" >/dev/null

  printf 'agent-install: source hook disable, concurrent Stop, bounded advisory, maintenance, hint, empty-result, marker-sweep, and advisory-race regressions passed\n'
}

test_source_hook_regressions
[ "${COMEMORY_TEST_SOURCE_HOOKS_ONLY:-0}" != 1 ] || exit 0

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
  jq -e '.hookSpecificOutput.additionalContext | contains("MCP find (k=3)")' "$TASK/start-$host.json" >/dev/null
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
  # A host without any configuration root must keep the installed hook silent.
  printf '%s' "$badge_payload" | env -u HOME -u CODEX_HOME -u CLAUDE_CONFIG_DIR \
    -u TOOLU_CONFIG_DIR -u TOOLU_HOST_OVERRIDE -u PLUGIN_ROOT -u CLAUDE_PLUGIN_ROOT \
    "$badge" > "$TASK/no-config-$host.out" 2> "$TASK/no-config-$host.err"
  [ ! -s "$TASK/no-config-$host.out" ]
  [ ! -s "$TASK/no-config-$host.err" ]
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

  # AC-11: session-end.sh launches Claude capture detached and returns within
  # the host's ~1.5s SessionEnd budget, with real payload and the real binary;
  # Codex exits silently because capture does not accept its transcript format,
  # even when this machine's own first-exec spawn latency is slow. Measured
  # twice (cold, then warm): first-exec-of-a-fresh-process latency on this
  # Mac is a known, separate, environmental cost from a hook that fails to
  # detach (see memories dc8b890a / f7c4b799), so only the WARM run is
  # asserted under budget. Every invocation is wrapped in `timeout 5` so a
  # detached child that still held this hook's stdout open would fail the
  # harness's own `$( … )` capture loudly (timeout kills it) instead of
  # hanging the test suite silently — proof the detach is real, not just a
  # background `&`. stdout+stderr are captured together and asserted empty.
  transcript="$TASK/transcript-$host.jsonl"
  : > "$transcript"
  end_payload=$(jq -nc --arg cwd "$TASK/repository" --arg sid "session-end-$host" --arg tp "$transcript" \
    '{cwd:$cwd,session_id:$sid,transcript_path:$tp,reason:"other",hook_event_name:"SessionEnd"}')
  for run in cold warm; do
    end_start=$(python3 -c 'import time; print(time.time())')
    end_out=$(printf '%s' "$end_payload" | timeout 5 "$plugin/hooks/session-end.sh" 2>&1) \
      || { printf 'session-end.sh %s run hung or failed (exit %s)\n' "$run" "$?" >&2; exit 1; }
    end_finish=$(python3 -c 'import time; print(time.time())')
    [ -z "$end_out" ] || { printf 'session-end.sh %s run produced output: %s\n' "$run" "$end_out" >&2; exit 1; }
    python3 - "$host" "$run" "$end_start" "$end_finish" <<'PYELAPSED'
import sys
host, run, start, finish = sys.argv[1], sys.argv[2], float(sys.argv[3]), float(sys.argv[4])
elapsed = finish - start
print(f"agent-install: {host} session-end.sh ({run}) returned in {elapsed:.3f}s")
if run == "warm":
    assert elapsed < 1.5, f"session-end.sh (warm) took {elapsed:.3f}s"
PYELAPSED
  done
  noc_end_out=$(printf '%s' "$end_payload" | timeout 5 env PATH=/usr/bin:/bin "$plugin/hooks/session-end.sh" 2>&1) \
    || { printf 'session-end.sh (no comemory on PATH) hung or failed (exit %s)\n' "$?" >&2; exit 1; }
  [ -z "$noc_end_out" ]

  # AC-12: comemory-status.sh nudges toward memory-bootstrap on a repo with
  # zero memories, and stays silent once one is saved.
  boot_repo="$TASK/bootstrap-$host"
  mkdir -p "$boot_repo"
  git init -q "$boot_repo"
  boot_payload=$(jq -nc --arg cwd "$boot_repo" '{cwd:$cwd}')
  printf '%s' "$boot_payload" | "$badge" | jq -e '.hookSpecificOutput.additionalContext | contains("memory-bootstrap")' >/dev/null
  (cd "$boot_repo" && "$wrapper" save "Bootstrap seed $host" "Verified bootstrap fixture" --json) >/dev/null
  boot_nudge_after=$(printf '%s' "$boot_payload" | "$badge")
  [ -z "$boot_nudge_after" ]

  # Write-once session-start marker: same session_id fired twice never moves
  # the recorded instant, matching resume/clear/compact re-firing SessionStart.
  ws_session="write-once-$host"
  ws_payload=$(jq -nc --arg cwd "$TASK/repository" --arg sid "$ws_session" '{cwd:$cwd,session_id:$sid,source:"startup",hook_event_name:"SessionStart"}')
  printf '%s' "$ws_payload" | "$plugin/hooks/session-start.sh" >/dev/null
  ws_marker="$cfg/comemory/session-$ws_session.start"
  [ -f "$ws_marker" ]
  ws_first=$(cat "$ws_marker")
  printf '%s' "$ws_payload" | "$plugin/hooks/session-start.sh" >/dev/null
  ws_second=$(cat "$ws_marker")
  [ "$ws_first" = "$ws_second" ]

  # 7-day sweep: session-start.sh prunes a stale session-*.start marker but
  # keeps a fresh one written by the same run.
  mkdir -p "$cfg/comemory"
  old_marker="$cfg/comemory/session-sweep-old-$host.start"
  printf '1970-01-01T00:00:00Z' > "$old_marker"
  eight_days_ago=$(date -v-8d +%Y%m%d%H%M 2>/dev/null || date -d '8 days ago' +%Y%m%d%H%M)
  touch -t "$eight_days_ago" "$old_marker"
  sweep_session="sweep-fresh-$host"
  sweep_payload=$(jq -nc --arg cwd "$TASK/repository" --arg sid "$sweep_session" '{cwd:$cwd,session_id:$sid,source:"startup",hook_event_name:"SessionStart"}')
  printf '%s' "$sweep_payload" | "$plugin/hooks/session-start.sh" >/dev/null
  [ ! -e "$old_marker" ]
  [ -f "$cfg/comemory/session-$sweep_session.start" ]

  # Session ids name marker files. ps_sanitize_session_id rejects any value
  # with a character outside [A-Za-z0-9_-] whole (empty output, non-zero)
  # rather than stripping it to a safe leftover, and passes a host UUID
  # untouched — asserted on the function itself, since the fixed `session-`
  # prefix means no crafted id can reach a `..` path segment on disk.
  uuid_sid="0f9d4c2e-1b2a-4c3d-8e7f-a1b2c3d4e5f6"
  ( . "$plugin/lib/project-skills-foundation.sh"
    if ps_sanitize_session_id "../../escape-$host" >/dev/null; then exit 1; fi
    [ "$(ps_sanitize_session_id "$uuid_sid")" = "$uuid_sid" ] )
  # And the hook writes no marker at all for a rejected id.
  slash_sid="../../escape-$host"
  slash_payload=$(jq -nc --arg cwd "$TASK/repository" --arg sid "$slash_sid" '{cwd:$cwd,session_id:$sid,source:"startup",hook_event_name:"SessionStart"}')
  printf '%s' "$slash_payload" | "$plugin/hooks/session-start.sh" >/dev/null
  leftover_escape=$(find "$TASK" -iname "*escape-$host*" 2>/dev/null)
  [ -z "$leftover_escape" ]

  # AC-9 / AC-10 fixture: a dedicated, otherwise-empty repo so recall
  # injection and the Stop advisory see exactly one relevant memory.
  rebase_repo="$TASK/rebase-$host"
  mkdir -p "$rebase_repo"
  git init -q "$rebase_repo"
  (cd "$rebase_repo" && "$wrapper" save "Rebase on main before push" 'Fetch and rebase origin/main before every push or PR.' --kind convention --json) > "$TASK/rebase-$host.json"
  rebase_id=$(jq -r '.id' "$TASK/rebase-$host.json")
  [ -n "$rebase_id" ] && [ "$rebase_id" != null ]
  rebase_key=$(basename "$rebase_repo")

  # AC-9: recall injection on UserPromptSubmit names the memory; skip list and
  # a missing comemory both stay silent; the injected find is never tracked.
  queries_before=$(comemory recall-status --repo "$rebase_key" --json | jq -r '.queries')
  push_payload=$(jq -nc --arg cwd "$rebase_repo" --arg prompt "how do I push this branch safely" \
    '{cwd:$cwd,hook_event_name:"UserPromptSubmit",prompt:$prompt}')
  push_out=$(printf '%s' "$push_payload" | "$plugin/hooks/memory-lifecycle.sh")
  jq -e --arg id "$rebase_id" '.hookSpecificOutput.additionalContext | contains($id)' <<<"$push_out" >/dev/null
  jq -e '.hookSpecificOutput.additionalContext | contains("Rebase on main before push")' <<<"$push_out" >/dev/null
  jq -e '.hookSpecificOutput.additionalContext
    | contains("selectively show")
      and contains("Feedback applies only to a separate tracked recall with a query_id")
      and (contains("then feedback") | not)
      and (contains("Call find") | not)' <<<"$push_out" >/dev/null
  ok_payload=$(jq -nc --arg cwd "$rebase_repo" '{cwd:$cwd,hook_event_name:"UserPromptSubmit",prompt:"ok"}')
  ok_out=$(printf '%s' "$ok_payload" | "$plugin/hooks/memory-lifecycle.sh")
  [ -z "$ok_out" ]
  noc_push_out=$(printf '%s' "$push_payload" | env PATH=/usr/bin:/bin "$plugin/hooks/memory-lifecycle.sh")
  [ -z "$noc_push_out" ]
  queries_after=$(comemory recall-status --repo "$rebase_key" --json | jq -r '.queries')
  [ "$queries_before" = "$queries_after" ]

  # Project-level `.claude/comemory.json` / `.codex/comemory.json` overrides
  # the user-level file: recall.inject true at user scope but false at the
  # repo's project config means no injection, even though $rebase_repo has a
  # matching saved memory that a working injection would otherwise surface.
  printf '%s\n' '{"recall":{"inject":true}}' > "$cfg/comemory.json"
  mkdir -p "$rebase_repo/.claude" "$rebase_repo/.codex"
  printf '%s\n' '{"recall":{"inject":false}}' > "$rebase_repo/.claude/comemory.json"
  printf '%s\n' '{"recall":{"inject":false}}' > "$rebase_repo/.codex/comemory.json"
  override_payload=$(jq -nc --arg cwd "$rebase_repo" --arg prompt "how do I push this branch safely" \
    '{cwd:$cwd,hook_event_name:"UserPromptSubmit",prompt:$prompt}')
  override_out=$(printf '%s' "$override_payload" | "$plugin/hooks/memory-lifecycle.sh")
  jq -e '.hookSpecificOutput.additionalContext | startswith("Comemory: use MCP find (k=3)")' <<<"$override_out" >/dev/null
  jq -e '.hookSpecificOutput.additionalContext | contains("Recall hint") | not' <<<"$override_out" >/dev/null
  rm -rf "$rebase_repo/.claude" "$rebase_repo/.codex"
  rm -f "$cfg/comemory.json"

  # recall.injectMinChars floor: a short, non-skip-list prompt gets no
  # injection attempt at all (just the plain reminder), even against a repo
  # with a matching saved memory.
  short_payload=$(jq -nc --arg cwd "$rebase_repo" --arg prompt "why is this slow" \
    '{cwd:$cwd,hook_event_name:"UserPromptSubmit",prompt:$prompt}')
  short_out=$(printf '%s' "$short_payload" | "$plugin/hooks/memory-lifecycle.sh")
  jq -e '.hookSpecificOutput.additionalContext | startswith("Comemory: use MCP find (k=3)")' <<<"$short_out" >/dev/null
  jq -e '.hookSpecificOutput.additionalContext | contains("Recall hint") | not' <<<"$short_out" >/dev/null

  # AC-10: Stop-time recall advisory. A repository window with one tracked,
  # judgeable recall emits a shared-activity systemMessage once and stays
  # silent after; stop_hook_active suppresses it; a recorded verdict suppresses
  # it; recall.enforce:false suppresses it.
  stop_a="stop-a-$host"
  start_a_payload=$(jq -nc --arg cwd "$rebase_repo" --arg sid "$stop_a" '{cwd:$cwd,session_id:$sid,source:"startup",hook_event_name:"SessionStart"}')
  printf '%s' "$start_a_payload" | "$plugin/hooks/session-start.sh" >/dev/null
  [ -f "$cfg/comemory/session-$stop_a.start" ]
  (cd "$rebase_repo" && "$wrapper" search rebase --json) > "$TASK/stop-search-$host.json"
  stop_qid=$(jq -r '.query_id' "$TASK/stop-search-$host.json")
  [ -n "$stop_qid" ] && [ "$stop_qid" != null ]
  stop_a_payload=$(jq -nc --arg cwd "$rebase_repo" --arg sid "$stop_a" '{cwd:$cwd,session_id:$sid,stop_hook_active:false,hook_event_name:"Stop"}')
  stop_a_out=$(printf '%s' "$stop_a_payload" | "$plugin/hooks/memory-lifecycle.sh")
  jq -e 'has("decision") | not' <<<"$stop_a_out" >/dev/null
  jq -e --arg id "$stop_qid" '.systemMessage | contains("shared repository activity") and contains($id)' <<<"$stop_a_out" >/dev/null
  stop_a_out2=$(printf '%s' "$stop_a_payload" | "$plugin/hooks/memory-lifecycle.sh")
  [ -z "$stop_a_out2" ]

  stop_b="stop-b-$host"
  stop_b_payload=$(jq -nc --arg cwd "$rebase_repo" --arg sid "$stop_b" '{cwd:$cwd,session_id:$sid,stop_hook_active:true,hook_event_name:"Stop"}')
  stop_b_out=$(printf '%s' "$stop_b_payload" | "$plugin/hooks/memory-lifecycle.sh")
  [ -z "$stop_b_out" ]

  stop_c="stop-c-$host"
  start_c_payload=$(jq -nc --arg cwd "$rebase_repo" --arg sid "$stop_c" '{cwd:$cwd,session_id:$sid,source:"startup",hook_event_name:"SessionStart"}')
  printf '%s' "$start_c_payload" | "$plugin/hooks/session-start.sh" >/dev/null
  qid_c=$(cd "$rebase_repo" && "$wrapper" search rebase --json | jq -r '.query_id')
  [ -n "$qid_c" ] && [ "$qid_c" != null ]
  comemory feedback "$qid_c" --used "$rebase_id" --json >/dev/null
  stop_c_payload=$(jq -nc --arg cwd "$rebase_repo" --arg sid "$stop_c" '{cwd:$cwd,session_id:$sid,stop_hook_active:false,hook_event_name:"Stop"}')
  stop_c_out=$(printf '%s' "$stop_c_payload" | "$plugin/hooks/memory-lifecycle.sh")
  [ -z "$stop_c_out" ]

  stop_d="stop-d-$host"
  start_d_payload=$(jq -nc --arg cwd "$rebase_repo" --arg sid "$stop_d" '{cwd:$cwd,session_id:$sid,source:"startup",hook_event_name:"SessionStart"}')
  printf '%s' "$start_d_payload" | "$plugin/hooks/session-start.sh" >/dev/null
  (cd "$rebase_repo" && "$wrapper" search rebase --json) >/dev/null
  printf '%s\n' '{"recall":{"enforce":false}}' > "$cfg/comemory.json"
  stop_d_payload=$(jq -nc --arg cwd "$rebase_repo" --arg sid "$stop_d" '{cwd:$cwd,session_id:$sid,stop_hook_active:false,hook_event_name:"Stop"}')
  stop_d_out=$(printf '%s' "$stop_d_payload" | "$plugin/hooks/memory-lifecycle.sh")
  [ -z "$stop_d_out" ]
  rm -f "$cfg/comemory.json"

  # AC-13: the shipped skills carry the tool catalog and the required
  # headings. Anchored on the backtick-quoted form the SKILL.md actually uses
  # (e.g. `` `find` ``) rather than a bare substring — "find" alone would
  # also match ordinary prose ("...find the memory...") and pass even if the
  # tool catalog entry were dropped.
  installed_agent_skill="$plugin/skills/agent-memory/SKILL.md"
  for tool in find search search_code context show list edges repos recall_status save feedback; do
    grep -qF -- "\`$tool\`" "$installed_agent_skill" \
      || { printf 'agent-memory SKILL.md missing backtick-quoted tool name: %s\n' "$tool" >&2; exit 1; }
  done
  grep -qF 'comemory.sh' "$installed_agent_skill"
  installed_bootstrap_skill="$plugin/skills/memory-bootstrap/SKILL.md"
  [ -f "$installed_bootstrap_skill" ]
  for heading in '## When to Use' '## Procedure' '## Pitfalls' '## Verification'; do
    grep -qF -- "$heading" "$installed_bootstrap_skill" \
      || { printf 'memory-bootstrap SKILL.md missing heading: %s\n' "$heading" >&2; exit 1; }
  done

  printf 'agent-install: %s native install/upgrade, SessionStart, worktree recall, scope denial passed\n' "$host"
  printf 'agent-install: %s SessionEnd behavior, bootstrap nudge, recall injection, and Stop advisory passed\n' "$host"
done

# AC-7: `.mcp.json` names this test binary's canonical path (`$BIN` above is
# itself a symlink into the real target/debug binary) and the `mcp`
# subcommand, for every host already installed above; `--dry-run` into a
# fresh config dir reports the path but writes nothing.
canonical_bin="$(python3 -c 'import os,sys;print(os.path.realpath(sys.argv[1]))' "$BIN")"
for host in claude codex; do
  plugin="$(jq -r .bundle "$TASK/install-$host.json")/plugins/comemory"
  jq -e --arg bin "$canonical_bin" \
    '.mcpServers.comemory.command == $bin and .mcpServers.comemory.args == ["mcp"]' \
    "$plugin/.mcp.json" >/dev/null
  dry_run_data="$TASK/$host data-dry-run"
  dry_run_cfg="$TASK/$host config-dry-run"
  "$BIN" install "$host" --data-dir "$dry_run_data" --config-dir "$dry_run_cfg" --dry-run --json > "$TASK/install-$host-dry-run.json"
  dry_run_manifest="$(jq -r .mcp_manifest "$TASK/install-$host-dry-run.json")"
  [ ! -e "$dry_run_manifest" ]
  printf 'agent-install: %s .mcp.json manifest command/args verified, dry-run wrote none\n' "$host"
done
