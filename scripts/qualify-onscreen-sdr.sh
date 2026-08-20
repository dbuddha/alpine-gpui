#!/bin/sh
set -eu

if test "$(uname -s)" != Darwin || test "$(uname -m)" != arm64; then
    printf 'onscreen SDR qualification requires Apple Silicon macOS\n' >&2
    exit 1
fi
if ! git diff --quiet || ! git diff --cached --quiet || test -n "$(git ls-files --others --exclude-standard)"; then
    printf 'onscreen SDR qualification requires a clean revision\n' >&2
    exit 1
fi

output=${1:-target/onscreen-sdr-qualification}
if test -e "$output"; then
    printf 'onscreen SDR output already exists: %s\n' "$output" >&2
    exit 1
fi
mkdir -p "$output"
output=$(cd "$output" && pwd)
helper=$(pwd)/target/onscreen-sdr-capture-helper
mkdir -p "$(dirname "$helper")"

xcrun swiftc -parse-as-library \
    tools/onscreen-sdr-capture/Capture.swift \
    -o "$helper"
"$helper" --self-test

revision=$(git rev-parse HEAD)
ALPINE_ONSCREEN_SDR_HELPER="$helper" \
ALPINE_ONSCREEN_SDR_OUTPUT="$output" \
ALPINE_ONSCREEN_SDR_REVISION="$revision" \
RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
    cargo test --locked -p alpine-platform-macos --test native_onscreen_sdr -- \
        --nocapture --test-threads=1

cargo run --quiet --locked -p alpine-assurance -- \
    validate-onscreen-sdr "$output"
cargo run --quiet --locked -p alpine-assurance -- \
    onscreen-sdr-report "$output" > "$output/report.md"
printf 'onscreen SDR qualification bundle: %s\n' "$output"
