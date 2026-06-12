#!/usr/bin/env bats
# search-code: index a real repo, retrieve a ranked hit (criterion 5).
load helper

@test "search-code: an indexed symbol is retrievable" {
    require_comemory
    repo="$(make_repo foo)"
    cat >"${repo}/lib.rs" <<'RUST'
/// A deliberately distinctive symbol for the plugin search-code test.
pub fn distinctive_widget_fn(count: u32) -> u32 {
    count.saturating_add(1)
}
RUST
    cd "$repo"
    git add -A
    # Indexing is maintenance setup (not a plugin surface) → use the raw binary.
    run comemory index-code --repo foo --path .
    [ "$status" -eq 0 ]

    # search-code IS the plugin surface → exercise it through the wrapper.
    run "$WRAPPER" search-code "distinctive_widget_fn" --json
    [ "$status" -eq 0 ]
    [[ "$output" == *distinctive_widget_fn* ]]
}
