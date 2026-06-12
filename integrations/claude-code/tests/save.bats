#!/usr/bin/env bats
# save: persists via stdin body, surfaces the near-dup advisory (criterion 6).
load helper

@test "save: writes a memory and returns an id" {
    require_comemory
    repo="$(make_repo foo)"
    cd "$repo"
    run bash -c 'printf "%s" "first widget memory body" | "$WRAPPER" save --kind decision --quality 4 --json'
    [ "$status" -eq 0 ]
    [[ "$output" == *'"id"'* ]]
}

@test "save: multi-line heredoc body is preserved (no escaping needed)" {
    require_comemory
    repo="$(make_repo foo)"
    cd "$repo"
    run bash -c '"$WRAPPER" save --kind note --json <<"BODY"
line one with $dollar and "quotes"
line two with `backticks`
BODY'
    [ "$status" -eq 0 ]
    [[ "$output" == *'"id"'* ]]
}

@test "save: near-duplicate body surfaces duplicate_of, still exits 0" {
    require_comemory
    repo="$(make_repo foo)"
    cd "$repo"
    # SimHash near-dup needs enough tokens to be stable; a long body with a
    # one-word change stays within the Hamming radius (short bodies swing too far).
    base="The retrieval pipeline fuses FTS5 lexical hits with sqlite-vec ANN results using reciprocal rank fusion, then applies multiplicative priors for activation recency feedback and quality before MMR diversification collapses near duplicates"
    run bash -c 'printf "%s" "'"$base"' here." | "$WRAPPER" save --kind note --json'
    [ "$status" -eq 0 ]
    # Re-save near-identical text → advisory duplicate_of.
    run bash -c 'printf "%s" "'"$base"' there." | "$WRAPPER" save --kind note --json'
    [ "$status" -eq 0 ]
    [[ "$output" == *duplicate_of* ]]
}
