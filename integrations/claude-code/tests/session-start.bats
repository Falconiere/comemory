#!/usr/bin/env bats
# SessionStart auto-recall digest, scoping, and fail-soft (criteria 1 & 2).
load helper

@test "session-start: digest shows this repo's memory, scoped" {
    require_comemory
    foo="$(make_repo foo)"
    bar="$(make_repo bar)"
    # One memory in each repo.
    ( cd "$foo" && printf "%s" "foo-only kappa note" | "$WRAPPER" save --kind decision --json )
    ( cd "$bar" && printf "%s" "bar-only omega note" | "$WRAPPER" save --kind note --json )

    cd "$foo"
    run bash "$HOOK"
    [ "$status" -eq 0 ]
    [ -n "$output" ]
    [[ "$output" == *foo* ]]
    [[ "$output" != *bar* ]]
}

@test "session-start: empty repo prints nothing, exit 0" {
    require_comemory
    empty="$(make_repo empty)"
    cd "$empty"
    run bash "$HOOK"
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

@test "session-start: missing binary prints nothing, exit 0" {
    foo="$(make_repo foo)"
    cd "$foo"
    PATH=/usr/bin:/bin run bash "$HOOK"
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}
