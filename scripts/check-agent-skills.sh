#!/bin/sh
set -eu
repo_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
if [ "$#" -eq 0 ]; then
    exec python3 "$repo_root/scripts/check-agent-skills.py"
fi
[ "$#" -eq 2 ] && [ "$1" = --skills-root ] || {
    printf 'usage: scripts/check-agent-skills.sh [--skills-root PATH]\n' >&2
    exit 2
}
exec python3 "$repo_root/scripts/check-agent-skills.py" "$2"
