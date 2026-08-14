#!/bin/sh
set -eu

mkdir -p target/assurance
cargo run --quiet --locked -p alpine-assurance -- validate
cargo run --quiet --locked -p alpine-assurance -- report \
    > target/assurance/report.md

fixture_output=target/assurance/duplicate-claim.log
if cargo run --quiet --locked -p alpine-assurance -- \
    validate assurance/fixtures/duplicate-claim.toml \
    >"$fixture_output" 2>&1; then
    printf 'invalid assurance fixture unexpectedly passed\n' >&2
    exit 1
fi

grep -Fq 'duplicate claim identifier AEP-0009-C01' "$fixture_output"

assert_fixture_fails() {
    fixture=$1
    expected=$2
    output=target/assurance/$(basename "$fixture" .toml).log
    if cargo run --quiet --locked -p alpine-assurance -- validate "$fixture" \
        >"$output" 2>&1; then
        printf 'invalid assurance fixture unexpectedly passed: %s\n' "$fixture" >&2
        exit 1
    fi
    grep -Fq "$expected" "$output"
}

assert_fixture_fails assurance/fixtures/missing-artifact.toml \
    'references missing artifact missing/evidence.rs'
assert_fixture_fails assurance/fixtures/kani-without-companion.toml \
    'needs an existing dynamic companion'
assert_fixture_fails assurance/fixtures/performance-without-benchmark.toml \
    'performance claim AEP-0009-C01 lacks a benchmark'
