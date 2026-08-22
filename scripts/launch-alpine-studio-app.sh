#!/bin/sh
set -eu

usage() {
    cat <<'EOF'
usage: scripts/launch-alpine-studio-app.sh [FILE_OR_FOLDER]

Launch the already-built local release Alpine Studio.app through LaunchServices.
EOF
}

if [ "${1-}" = --help ] || [ "${1-}" = -h ]; then
    usage
    exit 0
fi
if [ "$#" -gt 1 ]; then
    printf 'Alpine Studio launch error: expected at most one file or folder\n' >&2
    usage >&2
    exit 2
fi
if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
    printf 'Alpine Studio launch error: Apple Silicon macOS is required\n' >&2
    exit 1
fi

repository_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
bundle="$repository_root/target/release/Alpine Studio.app"
if [ ! -x "$bundle/Contents/MacOS/alpine-studio" ]; then
    printf 'Alpine Studio launch error: build the release app first with scripts/build-alpine-studio-app.sh\n' >&2
    exit 1
fi

if [ "$#" -eq 0 ]; then
    exec open -n "$bundle"
fi

requested=$1
if [ ! -e "$requested" ]; then
    printf 'Alpine Studio launch error: path does not exist: %s\n' "$requested" >&2
    exit 1
fi
requested_parent=$(CDPATH= cd -- "$(dirname "$requested")" && pwd -P)
requested_path="$requested_parent/$(basename "$requested")"
exec open -n "$bundle" --args "$requested_path"
