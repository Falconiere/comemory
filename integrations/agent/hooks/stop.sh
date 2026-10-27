#!/usr/bin/env bash
# Stop entry point for hosts that register each hook source once (muse
# rejects one script under two events): the Stop recall advisory and daily
# maintenance live in memory-lifecycle.sh, which dispatches on the payload's
# hook_event_name. Claude and Codex register memory-lifecycle.sh directly.
exec bash "${BASH_SOURCE%/*}/memory-lifecycle.sh"
