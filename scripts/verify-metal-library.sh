#!/bin/sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
library_file=${1:-"$repository_root/shaders/offscreen.metallib"}
manifest_file=${2:-"$repository_root/shaders/offscreen.metallib.manifest.txt"}
source_file=${3:-"$repository_root/shaders/offscreen.metal"}

fail() {
    printf 'Metal library verification error: %s\n' "$1" >&2
    exit 1
}

hash_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        fail 'sha256sum or shasum is required'
    fi
}

read_value() {
    key=$1
    count=$(grep -c "^${key}=" "$manifest_file" || true)
    if [ "$count" -ne 1 ]; then
        fail "manifest requires exactly one ${key} field"
    fi
    sed -n "s/^${key}=//p" "$manifest_file"
}

[ -s "$library_file" ] || fail "library is missing or empty: $library_file"
[ -s "$manifest_file" ] || fail "manifest is missing or empty: $manifest_file"
[ -s "$source_file" ] || fail "source is missing or empty: $source_file"

expected_keys='source
deployment_target
source_sha256
metallib_sha256
sdk_path
xcode_version
metal_version'
manifest_lines=$(wc -l < "$manifest_file" | tr -d ' ')
[ "$manifest_lines" -eq 7 ] \
    || fail "manifest requires exactly seven fields, found $manifest_lines"
actual_keys=$(sed -n 's/^\([^=][^=]*\)=.*$/\1/p' "$manifest_file")
if [ "$actual_keys" != "$expected_keys" ]; then
    fail 'manifest fields or ordering do not match the version 1 contract'
fi

recorded_source=$(read_value source)
recorded_target=$(read_value deployment_target)
recorded_source_hash=$(read_value source_sha256)
recorded_library_hash=$(read_value metallib_sha256)
sdk_path=$(read_value sdk_path)
xcode_version=$(read_value xcode_version)
metal_version=$(read_value metal_version)

[ "$recorded_source" = 'shaders/offscreen.metal' ] \
    || fail "unexpected source identity: $recorded_source"
[ "$recorded_target" = '15.0' ] \
    || fail "unexpected deployment target: $recorded_target"
[ -n "$sdk_path" ] || fail 'sdk_path must not be empty'
[ -n "$xcode_version" ] || fail 'xcode_version must not be empty'
[ -n "$metal_version" ] || fail 'metal_version must not be empty'

actual_source_hash=$(hash_file "$source_file")
[ "$recorded_source_hash" = "$actual_source_hash" ] \
    || fail "source hash mismatch: expected $recorded_source_hash, found $actual_source_hash"

actual_library_hash=$(hash_file "$library_file")
[ "$recorded_library_hash" = "$actual_library_hash" ] \
    || fail "library hash mismatch: expected $recorded_library_hash, found $actual_library_hash"

magic=$(od -An -tx1 -N4 "$library_file" | tr -d ' \n')
[ "$magic" = '4d544c42' ] || fail "invalid metallib magic: $magic"

printf 'Metal library verification passed: %s\n' "$library_file"
