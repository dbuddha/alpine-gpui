#!/bin/sh
set -eu

if [ "$#" -ne 4 ]; then
    printf 'usage: %s BASE_SUMMARY HEAD_SUMMARY LCOV BASE_SHA\n' "$0" >&2
    exit 2
fi

base_summary=$1
head_summary=$2
lcov_file=$3
base_sha=$4
head_sha=${ALPINE_HEAD_SHA:-HEAD}

for command in jq awk; do
    if ! command -v "$command" >/dev/null 2>&1; then
        printf 'coverage error: required command not found: %s\n' "$command" >&2
        exit 1
    fi
done

head_lines=$(jq -r '.data[0].totals.lines.percent' "$head_summary")
head_functions=$(jq -r '.data[0].totals.functions.percent' "$head_summary")

awk -v value="$head_lines" 'BEGIN { exit !(value + 0 >= 85) }' || {
    printf 'coverage error: workspace line coverage %.2f is below 85%%\n' "$head_lines" >&2
    exit 1
}

awk -v value="$head_functions" 'BEGIN { exit !(value + 0 >= 90) }' || {
    printf 'coverage error: workspace function coverage %.2f is below 90%%\n' "$head_functions" >&2
    exit 1
}

if [ -s "$base_summary" ]; then
    base_lines=$(jq -r '.data[0].totals.lines.percent' "$base_summary")
    awk -v base="$base_lines" -v head="$head_lines" 'BEGIN {
        base = int(base * 100 + 0.5) / 100
        head = int(head * 100 + 0.5) / 100
        exit !(head >= base)
    }' || {
        printf 'coverage error: line coverage regressed from %.2f%% to %.2f%%\n' "$base_lines" "$head_lines" >&2
        exit 1
    }
fi

critical_file_count=$(jq '[.data[0].files[] | select(.filename | test("/crates/alpine-(core|scene)/src/"))] | length' "$head_summary")
if [ "$critical_file_count" -lt 2 ]; then
    printf 'coverage error: expected core and scene critical-file evidence, found %s files\n' "$critical_file_count" >&2
    exit 1
fi

jq -r '.data[0].files[] | select(.filename | test("/crates/alpine-(core|scene)/src/")) | [.filename, .summary.lines.percent, .summary.functions.percent] | @tsv' "$head_summary" |
while IFS="$(printf '\t')" read -r filename lines functions; do
    awk -v value="$lines" 'BEGIN { exit !(value + 0 >= 95) }' || {
        printf 'coverage error: critical file %s has %.2f%% line coverage, requires 95%%\n' "$filename" "$lines" >&2
        exit 1
    }
    awk -v value="$functions" 'BEGIN { exit !(value + 0 >= 100) }' || {
        printf 'coverage error: critical file %s has %.2f%% function coverage, requires 100%%\n' "$filename" "$functions" >&2
        exit 1
    }
done

changed_lines=$(mktemp)
trap 'rm -f "$changed_lines"' EXIT HUP INT TERM

git diff --unified=0 "$base_sha...$head_sha" -- 'crates/**/*.rs' |
awk '
    /^\+\+\+ b\// { file = substr($0, 7); next }
    /^@@ / {
        split($0, fields, " ")
        added = fields[3]
        sub(/^\+/, "", added)
        split(added, span, ",")
        start = span[1] + 0
        count = (span[2] == "" ? 1 : span[2] + 0)
        for (offset = 0; offset < count; offset++) {
            print file ":" start + offset
        }
    }
' > "$changed_lines"

if [ -s "$changed_lines" ]; then
    changed_result=$(awk -F: '
        NR == FNR { changed[$0] = 1; next }
        /^SF:/ {
            source = substr($0, 4)
            sub(/^.*\/crates\//, "crates/", source)
            next
        }
        /^DA:/ {
            split(substr($0, 4), data, ",")
            key = source ":" data[1]
            if (changed[key]) {
                total++
                if (data[2] + 0 > 0) covered++
            }
        }
        END { printf "%d %d", covered + 0, total + 0 }
    ' "$changed_lines" "$lcov_file")
    changed_covered=$(printf '%s\n' "$changed_result" | awk '{print $1}')
    changed_total=$(printf '%s\n' "$changed_result" | awk '{print $2}')
    if [ "$changed_total" -gt 0 ]; then
        awk -v covered="$changed_covered" -v total="$changed_total" 'BEGIN { exit !(covered * 100 >= total * 90) }' || {
            printf 'coverage error: changed executable Rust lines covered %s/%s, requires 90%%\n' "$changed_covered" "$changed_total" >&2
            exit 1
        }
    fi
fi

printf 'coverage gates passed: lines %.2f%%, functions %.2f%%\n' "$head_lines" "$head_functions"
