#!/bin/sh
set -eu

hash_without_identity() {
    trace=$1
    if command -v sha256sum >/dev/null 2>&1; then
        sed '/^workload_hash = /d' "$trace" | sha256sum | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        sed '/^workload_hash = /d' "$trace" | shasum -a 256 | awk '{print $1}'
    else
        printf 'workload hash check requires sha256sum or shasum\n' >&2
        exit 1
    fi
}

if [ "$#" -eq 0 ]; then
    set -- assurance/qualification/v1/*scene.toml
fi

for trace in "$@"; do
    declared=$(sed -nE 's/^workload_hash = "([0-9a-f]{64})"$/\1/p' "$trace")
    actual=$(hash_without_identity "$trace")
    if [ -z "$declared" ] || [ "$declared" != "$actual" ]; then
        printf 'workload hash mismatch for %s: declared %s, actual %s\n' \
            "$trace" "${declared:-missing}" "$actual" >&2
        exit 1
    fi
done

printf 'scene workload hashes match canonical trace bytes\n'
