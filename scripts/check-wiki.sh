#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
"$repo_root/scripts/wiki.sh" validate "$repo_root"
printf 'Wiki mirror sources are valid.\n'
