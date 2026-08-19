#!/bin/sh
set -eu

if [ "$#" -ne 5 ]; then
    printf 'usage: %s BASE_SUMMARY HEAD_SUMMARY BASE_LCOV HEAD_LCOV BASE_SHA\n' "$0" >&2
    exit 2
fi

base_summary=$1
head_summary=$2
base_lcov_file=$3
lcov_file=$4
base_sha=$5
head_sha=${ALPINE_HEAD_SHA:-HEAD}

for command in jq awk sort; do
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
    if [ ! -s "$base_lcov_file" ]; then
        printf 'coverage error: base LCOV evidence is missing\n' >&2
        exit 1
    fi
    base_line_counts=$(awk -F'[:,]' '/^DA:/ { total++; if ($3 + 0 > 0) covered++ } END { printf "%d %d", covered + 0, total + 0 }' "$base_lcov_file")
    head_line_counts=$(awk -F'[:,]' '/^DA:/ { total++; if ($3 + 0 > 0) covered++ } END { printf "%d %d", covered + 0, total + 0 }' "$lcov_file")
    base_covered=$(printf '%s\n' "$base_line_counts" | awk '{print $1}')
    base_total=$(printf '%s\n' "$base_line_counts" | awk '{print $2}')
    head_covered=$(printf '%s\n' "$head_line_counts" | awk '{print $1}')
    head_total=$(printf '%s\n' "$head_line_counts" | awk '{print $2}')
    if [ "$base_total" -eq 0 ] || [ "$head_total" -eq 0 ]; then
        printf 'coverage error: concrete LCOV line evidence is empty\n' >&2
        exit 1
    fi
    awk -v base_covered="$base_covered" -v base_total="$base_total" -v head_covered="$head_covered" -v head_total="$head_total" 'BEGIN {
        exit !(head_covered * base_total >= base_covered * head_total)
    }' || {
        printf 'coverage error: concrete line coverage regressed from %s/%s to %s/%s\n' "$base_covered" "$base_total" "$head_covered" "$head_total" >&2
        exit 1
    }
fi

critical_file_count=$(jq '[.data[0].files[] | select(.filename | test("/crates/alpine-(core|scene|metal|platform)/src/"))] | length' "$head_summary")
if [ "$critical_file_count" -lt 6 ]; then
    printf 'coverage error: expected core, scene, Metal, and platform critical-file evidence, found %s files\n' "$critical_file_count" >&2
    exit 1
fi

jq -r '.data[0].files[] | select(.filename | test("/crates/alpine-(core|scene|metal|platform)/src/")) | [.filename, .summary.lines.percent, .summary.functions.percent] | @tsv' "$head_summary" |
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
uncovered_lines=$(mktemp)
trap 'rm -f "$changed_lines" "$uncovered_lines"' EXIT HUP INT TERM

git diff --unified=0 "$base_sha...$head_sha" -- 'crates/**/*.rs' 'apps/**/*.rs' |
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
    changed_result=$(awk -F: -v uncovered="$uncovered_lines" '
        NR == FNR { changed[$0] = 1; next }
        /^SF:/ {
            source = substr($0, 4)
            gsub(/\\/, "/", source)
            if (source ~ /(^|\/)crates\//) {
                sub(/^.*\/crates\//, "crates/", source)
            } else if (source ~ /(^|\/)apps\//) {
                sub(/^.*\/apps\//, "apps/", source)
            } else {
                source = ""
            }
            next
        }
        /^DA:/ {
            split(substr($0, 4), data, ",")
            key = source ":" data[1]
            if (changed[key] && !seen[key]++) {
                total++
                if (data[2] + 0 > 0) {
                    covered++
                } else {
                    print key > uncovered
                }
            }
        }
        END { printf "%d %d", covered + 0, total + 0 }
    ' "$changed_lines" "$lcov_file")
    changed_covered=$(printf '%s\n' "$changed_result" | awk '{print $1}')
    changed_total=$(printf '%s\n' "$changed_result" | awk '{print $2}')
    if [ "$changed_total" -gt 0 ]; then
        awk -v covered="$changed_covered" -v total="$changed_total" 'BEGIN { exit !(covered * 100 >= total * 90) }' || {
            LC_ALL=C sort -u "$uncovered_lines" |
            while IFS= read -r line; do
                printf 'coverage error: uncovered changed executable Rust line: %s\n' "$line" >&2
            done
            printf 'coverage error: changed executable Rust lines covered %s/%s, requires 90%%\n' "$changed_covered" "$changed_total" >&2
            exit 1
        }
    fi
fi

printf 'coverage gates passed: lines %.2f%%, functions %.2f%%\n' "$head_lines" "$head_functions"
