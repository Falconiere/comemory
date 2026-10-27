#!/usr/bin/env bash
# PreCompact entry point for hosts that register each hook source once (muse
# rejects one script under two events): the pre-compaction save reminder
# lives in memory-lifecycle.sh, which dispatches on the payload's
# hook_event_name. Claude and Codex register memory-lifecycle.sh directly.
exec bash "${BASH_SOURCE%/*}/memory-lifecycle.sh"
