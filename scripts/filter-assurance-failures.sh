#!/bin/sh
set -eu

input=$(mktemp)
trap 'rm -f "$input"' EXIT HUP INT TERM
cat > "$input"

if awk -F '\t' '$1 != "ci-pass" { found = 1 } END { exit !found }' "$input"; then
    awk -F '\t' '$1 != "ci-pass"' "$input"
else
    cat "$input"
fi
