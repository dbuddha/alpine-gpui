#!/bin/sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
fixture_directory=$(mktemp -d)
trap 'rm -rf -- "$fixture_directory"' EXIT HUP INT TERM

hash_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

write_manifest() {
    manifest_library_file=$1
    manifest_output_file=$2
    manifest_source_hash=$3
    {
        printf 'source=shaders/offscreen.metal\n'
        printf 'deployment_target=15.0\n'
        printf 'source_sha256=%s\n' "$manifest_source_hash"
        printf 'metallib_sha256=%s\n' "$(hash_file "$manifest_library_file")"
        printf 'sdk_path=/fixture/MacOSX.sdk\n'
        printf 'xcode_version=Xcode fixture\n'
        printf 'metal_version=Apple metal fixture\n'
    } > "$manifest_output_file"
}

source_file="$repository_root/shaders/offscreen.metal"
source_hash=$(hash_file "$source_file")
valid_library="$fixture_directory/valid.metallib"
valid_manifest="$fixture_directory/valid.manifest.txt"
cp "$repository_root/shaders/offscreen.metallib" "$valid_library"
write_manifest "$valid_library" "$valid_manifest" "$source_hash"
scripts/verify-metal-library.sh "$valid_library" "$valid_manifest" "$source_file" >/dev/null

duplicate_manifest="$fixture_directory/duplicate.manifest.txt"
cp "$valid_manifest" "$duplicate_manifest"
printf 'source_sha256=%s\n' "$source_hash" >> "$duplicate_manifest"
if scripts/verify-metal-library.sh "$valid_library" "$duplicate_manifest" "$source_file" \
    > "$fixture_directory/duplicate.log" 2>&1; then
    printf 'Metal library test error: duplicate manifest field passed\n' >&2
    exit 1
fi
grep -Fq 'manifest requires exactly seven fields' "$fixture_directory/duplicate.log"

bad_hash_manifest="$fixture_directory/bad-hash.manifest.txt"
write_manifest "$valid_library" "$bad_hash_manifest" \
    '0000000000000000000000000000000000000000000000000000000000000000'
if scripts/verify-metal-library.sh "$valid_library" "$bad_hash_manifest" "$source_file" \
    > "$fixture_directory/bad-hash.log" 2>&1; then
    printf 'Metal library test error: stale source hash passed\n' >&2
    exit 1
fi
grep -Fq 'source hash mismatch' "$fixture_directory/bad-hash.log"

bad_magic_library="$fixture_directory/bad-magic.metallib"
bad_magic_manifest="$fixture_directory/bad-magic.manifest.txt"
cp "$valid_library" "$bad_magic_library"
printf 'NOPE' | dd of="$bad_magic_library" bs=1 count=4 conv=notrunc 2>/dev/null
write_manifest "$bad_magic_library" "$bad_magic_manifest" "$source_hash"
if scripts/verify-metal-library.sh "$bad_magic_library" "$bad_magic_manifest" "$source_file" \
    > "$fixture_directory/bad-magic.log" 2>&1; then
    printf 'Metal library test error: invalid magic passed\n' >&2
    exit 1
fi
grep -Fq 'invalid metallib magic' "$fixture_directory/bad-magic.log"

printf 'Metal library verifier tests passed\n'
