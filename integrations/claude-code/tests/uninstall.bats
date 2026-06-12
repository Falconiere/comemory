#!/usr/bin/env bats
# uninstall.sh: prints guidance, and never deletes data without typed consent.
load helper

UNINSTALL="${PLUGIN_ROOT:-}/scripts/uninstall.sh"

@test "uninstall: no args prints guidance and keeps data" {
    UNINSTALL="$(cd "${BATS_TEST_DIRNAME}/.." && pwd)/scripts/uninstall.sh"
    # COMEMORY_DATA_DIR (a real temp dir) exists from setup; seed a memory file.
    : >"${COMEMORY_DATA_DIR}/marker"
    run bash "$UNINSTALL"
    [ "$status" -eq 0 ]
    [[ "$output" == *"/plugin"* ]]
    [[ "$output" == *"$COMEMORY_DATA_DIR"* ]]
    [ -f "${COMEMORY_DATA_DIR}/marker" ]   # data untouched
}

@test "uninstall: bad arg exits 64" {
    UNINSTALL="$(cd "${BATS_TEST_DIRNAME}/.." && pwd)/scripts/uninstall.sh"
    run bash "$UNINSTALL" --bogus
    [ "$status" -eq 64 ]
}

@test "uninstall: --purge-data with non-matching confirmation keeps data" {
    UNINSTALL="$(cd "${BATS_TEST_DIRNAME}/.." && pwd)/scripts/uninstall.sh"
    : >"${COMEMORY_DATA_DIR}/marker"
    run bash -c 'printf "%s\n" "definitely-not-the-path" | bash "$0" --purge-data' "$UNINSTALL"
    [ "$status" -eq 0 ]
    [ -d "$COMEMORY_DATA_DIR" ]
    [ -f "${COMEMORY_DATA_DIR}/marker" ]
}

@test "uninstall: --purge-data with matching confirmation deletes data" {
    UNINSTALL="$(cd "${BATS_TEST_DIRNAME}/.." && pwd)/scripts/uninstall.sh"
    : >"${COMEMORY_DATA_DIR}/marker"
    run bash -c 'printf "%s\n" "$1" | bash "$0" --purge-data' "$UNINSTALL" "$COMEMORY_DATA_DIR"
    [ "$status" -eq 0 ]
    [ ! -d "$COMEMORY_DATA_DIR" ]
}
