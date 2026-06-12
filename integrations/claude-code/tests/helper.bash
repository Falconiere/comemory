# helper.bash — shared bats fixtures. Real comemory binary, real temp git repos,
# isolated temp COMEMORY_DATA_DIR per test. No mocks.

PLUGIN_ROOT="$(cd "${BATS_TEST_DIRNAME}/.." && pwd)"
export WRAPPER="${PLUGIN_ROOT}/scripts/comemory.sh"
export HOOK="${PLUGIN_ROOT}/hooks/session-start.sh"
export UNINSTALL="${PLUGIN_ROOT}/scripts/uninstall.sh"

setup() {
    COMEMORY_DATA_DIR="$(mktemp -d)"
    export COMEMORY_DATA_DIR
    TEST_TMP="$(mktemp -d)"
    export TEST_TMP
}

teardown() {
    rm -rf "$COMEMORY_DATA_DIR" "$TEST_TMP"
}

# make_repo NAME — create a fresh git repo whose basename == NAME; echoes path.
make_repo() {
    local name="$1"
    local dir="${TEST_TMP}/${name}"
    mkdir -p "$dir"
    git -C "$dir" init -q
    git -C "$dir" config user.email t@e.st
    git -C "$dir" config user.name tester
    printf '%s\n' "$dir"
}

# require_comemory — skip the test when the binary isn't installed.
require_comemory() {
    command -v comemory >/dev/null 2>&1 || skip "comemory binary not on PATH"
}
