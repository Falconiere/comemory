#!/usr/bin/env bash
# Reject forbidden patterns in new content for Edit/Write/MultiEdit on src/*.rs.
# Pattern list mirrors scripts/no-bypass-check.sh — keep in sync.

: "${tool_name:=}"
: "${input:=}"

[[ "$tool_name" != "Edit" && "$tool_name" != "Write" && "$tool_name" != "MultiEdit" ]] && exit 0
HOOK_LIB="$(cd "$(dirname "$0")/../.." && pwd)/lib/common.sh"
# shellcheck source=../../lib/common.sh
source "$HOOK_LIB"

new_content=$(echo "$input" | jq -r '
  .tool_input.new_string //
  .tool_input.content //
  (.tool_input.edits // [] | map(.new_string) | join("\n")) //
  empty
' 2>/dev/null)
[[ -z "$new_content" ]] && exit 0

file_path=$(echo "$input" | jq -r '.tool_input.file_path // empty' 2>/dev/null)
case "$file_path" in
  */src/*.rs) ;;
  *) exit 0 ;;
esac

violations=""
add() { violations="${violations}"$'\n'"  - $1"; }

echo "$new_content" | grep -qE '#\[allow\('                                      && add "#[allow(...)] override"
echo "$new_content" | awk '
  /#\[cfg\(test\)\]/                                          { flagged = NR }
  /^[[:space:]]*mod[[:space:]]+tests([[:space:]]|\{|$)/       { if (flagged > 0 && NR - flagged <= 2) { found = 1; exit } }
  END                                                         { exit found ? 0 : 1 }
' && add "#[cfg(test)] mod tests inside src/ (move to tests/)"
echo "$new_content" | grep -qE '\.unwrap\(\)'                                    && add ".unwrap() in src/"
echo "$new_content" | grep -qE '(^|[^a-zA-Z0-9_])println!'                       && add "println! (use tracing)"
echo "$new_content" | grep -qE '(^|[^a-zA-Z0-9_])eprintln!'                      && add "eprintln! (use tracing)"
echo "$new_content" | grep -qE '(^|[^a-zA-Z0-9_])todo!\('                        && add "todo!()"
echo "$new_content" | grep -qE '(^|[^a-zA-Z0-9_])unimplemented!\('               && add "unimplemented!()"
echo "$new_content" | grep -qE '(^|[^a-zA-Z0-9_])panic!\('                       && add "panic!() in src/"

# unsafe { … } requires `// SAFETY:` within the 3 lines ABOVE (matches
# scripts/no-bypass-check.sh which scans upward from each unsafe site).
# Portable word boundary: (^|[^a-zA-Z0-9_]) — BSD awk has no \b/\<.
if echo "$new_content" | grep -qE '(^|[^a-zA-Z0-9_])unsafe[[:space:]]*\{'; then
  if ! echo "$new_content" | awk '
    /SAFETY:/                                          { last_safety = NR }
    /(^|[^a-zA-Z0-9_])unsafe[[:space:]]*\{/            {
      found = 1
      if (last_safety > 0 && NR - last_safety <= 3) {
        ok = 1
      } else {
        bad = 1
        exit 1
      }
    }
    END                                                { exit (bad || (found && !ok)) ? 1 : 0 }
  '; then
    add "unsafe { … } without // SAFETY: comment within 3 lines above"
  fi
fi

if [[ -n "$violations" ]]; then
  reason="Forbidden pattern(s) in ${file_path}:${violations}"$'\n'"Fix the root cause."
  deny_pre "$reason"
  exit 0
fi
exit 0
