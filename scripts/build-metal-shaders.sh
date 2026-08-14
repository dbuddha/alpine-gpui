#!/bin/sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
source_file="$repository_root/shaders/offscreen.metal"
output_file=${1:-"$repository_root/target/metal/offscreen.metallib"}
manifest_file="$output_file.manifest.txt"
temporary_directory=$(mktemp -d)
deployment_target=${MACOSX_DEPLOYMENT_TARGET:-15.0}
export MACOSX_DEPLOYMENT_TARGET="$deployment_target"

cleanup() {
    rm -rf -- "$temporary_directory"
}
trap cleanup EXIT HUP INT TERM

if ! command -v xcrun >/dev/null 2>&1; then
    printf 'Metal shader build error: xcrun is required\n' >&2
    exit 1
fi

metal_compiler=$(xcrun --sdk macosx --find metal 2>/dev/null || true)
metallib_linker=$(xcrun --sdk macosx --find metallib 2>/dev/null || true)
if [ -z "$metal_compiler" ] || [ -z "$metallib_linker" ]; then
    printf 'Metal shader build error: install the Xcode Metal Toolchain component\n' >&2
    exit 1
fi

mkdir -p -- "$(dirname -- "$output_file")"
air_file="$temporary_directory/offscreen.air"

"$metal_compiler" -c "$source_file" -o "$air_file"
"$metallib_linker" "$air_file" -o "$output_file"

if [ ! -s "$output_file" ]; then
    printf 'Metal shader build error: compiler produced no library\n' >&2
    exit 1
fi

{
    printf 'source=shaders/offscreen.metal\n'
    printf 'deployment_target=%s\n' "$deployment_target"
    printf 'source_sha256='
    shasum -a 256 "$source_file" | awk '{print $1}'
    printf 'metallib_sha256='
    shasum -a 256 "$output_file" | awk '{print $1}'
    printf 'sdk_path='
    xcrun --sdk macosx --show-sdk-path
    printf 'xcode_version='
    xcodebuild -version | tr '\n' ' ' | sed 's/ $/\n/'
    printf 'metal_version='
    "$metal_compiler" --version 2>&1 | head -n 1
} > "$manifest_file"

printf 'Built %s\n' "$output_file"
