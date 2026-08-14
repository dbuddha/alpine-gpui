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
assert_output "$docs" tla=false

core=$(run_fixture crates/alpine-core/src/lib.rs)
assert_output "$core" coverage=true
assert_output "$core" mutation=true
assert_output "$core" kani=true

formal=$(run_fixture formal/tla/aep-0009/AssuranceLifecycle.tla)
assert_output "$formal" tla=true
assert_output "$formal" kani=false

qualification=$(run_fixture assurance/qualification/v1/valid.toml)
assert_output "$qualification" coverage=true
assert_output "$qualification" tla=true
assert_output "$qualification" mutation=true
assert_output "$qualification" kani=false

assurance=$(run_fixture tools/alpine-assurance/src/qualification.rs)
assert_output "$assurance" coverage=true
assert_output "$assurance" mutation=true
assert_output "$assurance" tla=true

unsafe=$(run_fixture README.md review:unsafe)
assert_output "$unsafe" miri=true

metal=$(run_fixture crates/alpine-metal/src/lib.rs)
assert_output "$metal" coverage=true
assert_output "$metal" mutation=true
assert_output "$metal" kani=true
assert_output "$metal" metal=true

shader=$(run_fixture shaders/offscreen.metal)
assert_output "$shader" coverage=false
assert_output "$shader" mutation=false
assert_output "$shader" kani=false
assert_output "$shader" metal=true

printf 'CI classifier tests passed\n'
