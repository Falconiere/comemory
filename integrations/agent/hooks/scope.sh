#!/usr/bin/env bash
# Pre-tool check: enforce --repo scope on raw `comemory` CLI invocations.
#
# Without --repo, `comemory search` / `save` / `context` / `search-code` mix
# results across every repo in the local comemory store — wasted tokens at
# best, wrong-repo answers at worst. The `skills/agent-memory/scripts/comemory.sh
# <subcmd>` wrapper (resolved relative to this module — skills/ is a
# sibling of hooks/ inside the plugin root, in-repo and installed alike)
# auto-scopes; this module pushes the agent toward that path by denying
# unscoped raw calls.
#
# Subcommands that require scoping: search, save, context, search-code,
# index-code, graph. `context` is a real comemory verb (a repo-scoped headline
# lookup) and so is guarded here even though the comemory.sh wrapper does not
# dispatch it — a raw `comemory context` without --repo would still leak across
# repos. index-code/graph accept --repo (the wrapper auto-injects it), so a raw
# unscoped call carries the same leak hazard.
# The retrieval-loop verbs (mine, tune, eval, prune, gc, rebuild, feedback)
# and list/doctor/stats/serve/--help/--version are intentionally global —
# comemory accepts no --repo on them.
#
# Always allowed: wrapper calls (`comemory.sh …` adds --repo itself), and
# raw calls that already include --repo. comemory has no `-p` short flag and no
# repo env var, so --repo is the only scope signal.
#
# Inputs (from parent dispatcher pre-tools/mod.sh, via `export`):
#   $tool_name - name of the tool being invoked
#   $input     - raw JSON payload (stdin also delivers it)

set -euo pipefail
HOOK_DIR="$(cd "${BASH_SOURCE%/*}" && pwd)"
# shellcheck source=../lib/project-skills.sh
. "$HOOK_DIR/../lib/project-skills.sh"
# shellcheck source=../lib/shell-input.sh
. "$HOOK_DIR/../lib/shell-input.sh"
input=$(cat)
command -v jq >/dev/null 2>&1 || exit 0
PS_CWD=$(jq -r '.cwd // empty' <<<"$input" 2>/dev/null) || exit 0
ps_memory_enabled || exit 0
tool_name=$(jq -r '.tool_name // empty' <<<"$input" 2>/dev/null) || exit 0
case "$tool_name" in Bash|exec_command|functions.exec_command) ;; *) exit 0 ;; esac

command=$(echo "$input" | jq -r '.tool_input.command // .tool_input.cmd // ""')
[[ -z "$command" ]] && exit 0

cmd_only=$(printf '%s\n' "$command" | strip_heredocs)

# NOTE: wrapper calls (`comemory.sh …`) are skipped PER SEGMENT inside the
# loop below — NOT here against the whole command. A whole-command skip let a
# single `comemory.sh` token (even in an `echo`, a comment, or one arm of
# `comemory.sh list && comemory search foo`) short-circuit enforcement for
# every other statement, so an unscoped raw `comemory search` slipped past.

# Split on shell statement separators (;, &&, ||) — but ONLY when they are
# unquoted. A naive `tr ';&|' '\n'` would split `comemory save "title; body"`
# inside the quoted argument, falsely denying a legitimate call. Use python3
# to walk the command character-by-character respecting single/double quotes
# and backslash escapes; fall back to the naive split if python3 is missing.
split_statements() {
  if command -v python3 >/dev/null 2>&1; then
    python3 - "$1" <<'PY' 2>/dev/null
import sys, shlex, re
s = sys.argv[1]
segments, buf = [], []
i, n = 0, len(s)
quote = None  # current quote char or None
while i < n:
    c = s[i]
    if quote:
        buf.append(c)
        if c == '\\' and i + 1 < n:
            buf.append(s[i + 1]); i += 2; continue
        if c == quote: quote = None
        i += 1; continue
    if c in ("'", '"'):
        quote = c; buf.append(c); i += 1; continue
    if c == '\\' and i + 1 < n:
        buf.append(c); buf.append(s[i + 1]); i += 2; continue
    # Unquoted separator?
    if c == ';':
        segments.append(''.join(buf)); buf = []; i += 1; continue
    if c in ('&', '|') and i + 1 < n and s[i + 1] == c:
        segments.append(''.join(buf)); buf = []; i += 2; continue
    buf.append(c); i += 1
if buf: segments.append(''.join(buf))
def normalize(segment):
    try:
        parts = shlex.split(segment, comments=True)
    except ValueError:
        return segment
    changed = False
    while parts:
        if re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", parts[0]):
            parts.pop(0)
            changed = True
            continue
        launcher = parts[0].rsplit("/", 1)[-1]
        if launcher == "command":
            parts.pop(0)
            if parts and parts[0] in ("-v", "-V"):
                return ""  # name lookup, not execution
            if parts and parts[0] == "-p": parts.pop(0)
            if parts and parts[0] == "--": parts.pop(0)
        elif launcher == "env":
            parts.pop(0)
            while parts and parts[0].startswith("-"):
                option = parts.pop(0)
                if option == "--": break
                if option in ("-u", "--unset", "-C", "--chdir"):
                    if not parts: return ""
                    parts.pop(0)
                elif option not in ("-i", "--ignore-environment", "-") and not option.startswith(("--unset=", "--chdir=")):
                    return segment  # unsupported env option; do not guess
        else:
            break
        changed = True
    return " ".join(shlex.quote(part) for part in parts) if changed else segment

for seg in segments:
    seg = normalize(seg.strip())
    if seg: print(seg)
PY
    return
  fi
  # Fallback: naive split. Accepts the quoted-string false-positive risk only
  # when python3 is unavailable.
  printf '%s\n' "$1" | tr ';&|' '\n' | sed '/^$/d'
}

# Then inspect each segment for a raw `comemory <subcmd>` invocation.
violation=""
while IFS= read -r segment; do
  # Trim leading whitespace + strip leading env-var assignments.
  segment="${segment#"${segment%%[![:space:]]*}"}"
  # The value may be a single/double-quoted string (which can contain
  # whitespace) OR a run of non-space chars. Matching only the latter would
  # stop at the first space inside a quoted value (MY_VAR="foo bar"), leaving
  # the tail (bar" comemory save) as the segment and letting an unscoped raw
  # comemory call slip past the ^comemory check below. The value alternation
  # is wrapped in a group, so the trailing REST is BASH_REMATCH[2].
  #
  # comemory has no repo env var, so an env prefix is never scope — it is just
  # stripped so the bare `comemory <subcmd>` underneath is inspected.
  #
  # The regex is built in a variable (with \047 = single quote) so the literal
  # single-quote of the quoted-value alternation never sits inside [[ ]] — an
  # embedded ' there desyncs shellcheck's parser.
  _env_re=$'^[A-Za-z_][A-Za-z0-9_]*=("[^"]*"|\047[^\047]*\047|[^[:space:]]+)[[:space:]]+(.*)$'
  while [[ "$segment" =~ $_env_re ]]; do
    # :- guard: defensive — if the capture group is somehow unset (regex
    # engine quirk), drop the segment instead of erroring under `set -u`.
    segment="${BASH_REMATCH[2]:-}"
  done
  [[ -z "$segment" ]] && continue

  # Wrapper call? `comemory.sh …` (optionally path-prefixed) auto-scopes —
  # skip THIS segment only. Evaluated per-segment so a wrapper call in one arm
  # of a chain never excuses a raw unscoped call in another.
  if [[ "$segment" =~ ^([^[:space:]]*/)?comemory\.sh([[:space:]]|$) ]]; then
    continue
  fi

  # Only raw `comemory <subcmd>` (not a path containing the literal `comemory`).
  if [[ ! "$segment" =~ ^comemory[[:space:]]+([a-z][a-zA-Z0-9_-]*) ]]; then
    continue
  fi
  subcmd="${BASH_REMATCH[1]:-}"
  [[ -z "$subcmd" ]] && continue

  # Only the repo-scoped verbs require --repo; everything else is global
  # (comemory accepts no --repo on the retrieval-loop verbs). index-code and
  # graph accept --repo and the comemory.sh wrapper auto-injects it for them, so a
  # raw unscoped call carries the same cross-repo-leak hazard and is guarded.
  case "$subcmd" in
    search|save|context|search-code|index-code|graph) ;;
    *) continue ;;
  esac

  # Already scoped via --repo? (comemory has no -p short flag and no repo env.)
  if [[ "$segment" =~ (^|[[:space:]])--repo([[:space:]]|=) ]]; then
    continue
  fi

  violation="$segment"
  break
done < <(split_statements "$cmd_only")

if [[ -n "$violation" ]]; then
  # Point the agent at the STABLE published path SessionStart register.sh
  # symlinks into the active host's config root at comemory/comemory.sh.
  # The plugin-root path (skills/agent-memory/scripts/comemory.sh) is unreachable
  # from the agent's Bash tool because ${CLAUDE_PLUGIN_ROOT} is not exported
  # there — quoting that path in the deny message would just re-trigger the
  # "empty results = not-found" failure mode this hook is trying to redirect.
  #
  # Always emit the SHELL TEMPLATE (single-quoted so the host variables stay
  # literal in the deny text and expand at the agent's shell, not here).
  wrapper="\"$(ps_config_root)/comemory/comemory.sh\""
  jq -n --arg cmd "$violation" --arg wrapper "$wrapper" '{
    "hookSpecificOutput": {
      "hookEventName": "PreToolUse",
      "permissionDecision": "deny",
      "permissionDecisionReason": (
        "comemory call missing --repo scope:\n  " + $cmd + "\n\n" +
        "comemory stores memories across multiple repos. Without --repo, search/save/context/search-code/index-code/graph leak across repos.\n\n" +
        "Fix one of:\n" +
        "  1. Prefer the wrapper (auto-scopes):\n" +
        "       " + $wrapper + " <subcmd> …\n" +
        "  2. Add --repo <name> to the raw call:\n" +
        "       comemory <subcmd> … --repo <name>"
      )
    }
  }'
  exit 0
fi

exit 0
