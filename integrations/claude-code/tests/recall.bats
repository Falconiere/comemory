#!/usr/bin/env bats
# recall: save → context round-trip on the real binary (criterion 4).
load helper

@test "recall: a saved memory is retrievable by its distinctive token" {
    require_comemory
    repo="$(make_repo foo)"
    cd "$repo"
    run bash -c 'printf "%s" "retrieval target zeta-quark convention" | "$WRAPPER" save --kind convention --json'
    [ "$status" -eq 0 ]

    run "$WRAPPER" context "zeta-quark" --json
    [ "$status" -eq 0 ]
    [[ "$output" == *zeta-quark* ]]
}
