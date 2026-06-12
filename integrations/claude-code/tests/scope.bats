#!/usr/bin/env bats
# Scope injection + missing-binary fail-soft (acceptance criteria 2 & 3).
load helper

@test "scope: inside a git repo, list is scoped to the repo basename" {
    require_comemory
    repo="$(make_repo foo)"
    cd "$repo"
    run bash -c 'printf "%s" "scoped widget memory" | "$WRAPPER" save --kind note --json'
    [ "$status" -eq 0 ]
    run "$WRAPPER" list
    [ "$status" -eq 0 ]
    [[ "$output" == *foo* ]]
}

@test "scope: COMEMORY_REPO overrides the git basename" {
    require_comemory
    repo="$(make_repo foo)"
    cd "$repo"
    run bash -c 'printf "%s" "scoped widget memory" | "$WRAPPER" save --kind note --json'
    [ "$status" -eq 0 ]
    # A different scope must not see the foo-scoped memory.
    COMEMORY_REPO=bar run "$WRAPPER" list
    [ "$status" -eq 0 ]
    [[ "$output" != *"scoped widget memory"* ]]
}

@test "scope: outside any git repo falls back to 'unknown'" {
    require_comemory
    cd "$TEST_TMP"   # mktemp dir, not a git repo
    # Save with no git scope, then assert the memory is filed under "unknown"
    # (exit code alone would pass even if scoping were silently skipped).
    run bash -c 'printf "%s" "rootless scope memory" | "$WRAPPER" save --kind note --json'
    [ "$status" -eq 0 ]
    run "$WRAPPER" list
    [ "$status" -eq 0 ]
    [[ "$output" == *unknown* ]]
}

@test "fail-soft: missing binary emits the unavailable sentinel, exit 0" {
    PATH=/usr/bin:/bin run bash "$WRAPPER" list
    [ "$status" -eq 0 ]
    [[ "$output" == *'"comemory":"unavailable"'* ]]
}

@test "scope: a leading flag is refused, not run unscoped" {
    require_comemory
    repo="$(make_repo foo)"
    cd "$repo"
    # A global flag before the subcommand would otherwise fall through to the
    # unscoped passthrough and silently skip --repo injection.
    run "$WRAPPER" --json list
    [ "$status" -eq 64 ]
    [[ "$output" == *"subcommand must precede flags"* ]]
}
