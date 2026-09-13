#!/bin/sh
set -eu

# This is a conservative emptiness proof for --in-diff, not discovery of all
# mutants. A changed Rust path always keeps the existing complete shard set.
[ "$#" -eq 3 ] || { printf 'usage: %s selected base head\n' "$0" >&2; exit 2; }
selected=$1
base=$2
head=$3
case "$selected" in true|false) ;; *) exit 2 ;; esac
for revision in "$base" "$head"; do
    case "$revision" in ''|*[!0-9a-f]*) exit 2 ;; esac
    [ "${#revision}" -eq 40 ] || exit 2
    git cat-file -e "$revision^{commit}"
done
rust_paths=$(git -c core.quotepath=true diff --no-renames --name-only "$base...$head" -- '*.rs')
required=false
reason=not-selected
if [ "$selected" = true ]; then
    if [ -n "$rust_paths" ]; then
        required=true
        reason=changed-rust-paths
    else
        reason=proven-empty-no-rust-paths
    fi
fi
printf 'diff mutation: %s; base=%s; head=%s; required=%s\n' "$reason" "$base" "$head" "$required" >&2
if [ -n "$GITHUB_OUTPUT" ]; then
    printf 'required=%s\nreason=%s\n' "$required" "$reason" >> "$GITHUB_OUTPUT"
else
    printf 'required=%s\nreason=%s\n' "$required" "$reason"
fi
