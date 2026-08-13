#!/bin/sh
set -eu

run_fixture() {
    output_file=$(mktemp)
    GITHUB_OUTPUT=$output_file \
    ALPINE_BASE_SHA=HEAD \
    ALPINE_HEAD_SHA=HEAD \
    ALPINE_CHANGED_FILES=$1 \
    ALPINE_PR_LABELS=${2:-} \
    scripts/classify-ci.sh
    cat "$output_file"
}

assert_output() {
    output=$1
    expected=$2
    if ! printf '%s\n' "$output" | grep -Fxq "$expected"; then
        printf 'classifier test error: expected %s\n%s\n' "$expected" "$output" >&2
        exit 1
    fi
}

docs=$(run_fixture README.md)
assert_output "$docs" coverage=false
assert_output "$docs" mutation=false
assert_output "$docs" kani=false

core=$(run_fixture crates/alpine-core/src/lib.rs)
assert_output "$core" coverage=true
assert_output "$core" mutation=true
assert_output "$core" kani=true

unsafe=$(run_fixture README.md review:unsafe)
assert_output "$unsafe" miri=true

metal=$(run_fixture crates/alpine-metal/src/lib.rs)
assert_output "$metal" coverage=true
assert_output "$metal" metal=true

printf 'CI classifier tests passed\n'
