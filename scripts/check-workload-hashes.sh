#!/bin/sh
set -eu

hash_stdin() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 | awk '{print $1}'
    else
        printf 'workload hash check requires sha256sum or shasum\n' >&2
        exit 1
    fi
}

hash_without_identity() {
    trace=$1
    sed '/^workload_hash = /d' "$trace" | hash_stdin
}

hash_a8_payload() {
    trace=$1
    if ! command -v perl >/dev/null 2>&1; then
        printf 'A8 content hash check requires perl\n' >&2
        exit 1
    fi
    perl -ne 'if (/^pixels = \[(.*)\]$/) { print pack("C*", split /, /, $1) }' \
        "$trace" | hash_stdin
}

if [ "$#" -eq 0 ]; then
    set -- assurance/qualification/v1/*scene.toml assurance/qualification/v2/*.toml
fi

for trace in "$@"; do
    declared=$(sed -nE 's/^workload_hash = "([0-9a-f]{64})"$/\1/p' "$trace")
    actual=$(hash_without_identity "$trace")
    if [ -z "$declared" ] || [ "$declared" != "$actual" ]; then
        printf 'workload hash mismatch for %s: declared %s, actual %s\n' \
            "$trace" "${declared:-missing}" "$actual" >&2
        exit 1
    fi

    pixel_lines=$(grep -c '^pixels = ' "$trace" || true)
    if [ "$pixel_lines" -gt 0 ]; then
        declared_content=$(sed -nE 's/^content_hash = "([0-9a-f]{64})"$/\1/p' "$trace")
        actual_content=$(hash_a8_payload "$trace")
        if [ "$pixel_lines" -ne 1 ] || [ -z "$declared_content" ] || \
            [ "$declared_content" != "$actual_content" ]; then
            printf 'A8 content hash mismatch for %s: declared %s, actual %s\n' \
                "$trace" "${declared_content:-missing}" "$actual_content" >&2
            exit 1
        fi
    fi
done

printf 'scene workload hashes match canonical trace bytes\n'
