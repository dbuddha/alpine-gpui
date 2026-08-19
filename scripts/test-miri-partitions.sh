#!/bin/sh
set -eu

manifest=assurance/miri-studio-partitions.tsv

awk -F '\t' '
    NF != 2 || $1 !~ /^[A-Za-z0-9_:]+$/ || $1 !~ /::/ || $2 !~ /^studio-/ { exit 1 }
    seen[$1]++ { if (seen[$1] != 1) exit 1 }
' "$manifest" || {
    printf 'Studio Miri manifest is malformed or contains duplicate prefixes.\n' >&2
    exit 1
}

tests=$(cargo test --locked -p alpine-studio --lib -- --list 2>/dev/null \
    | sed -n 's/: test$//p')
if [ -z "$tests" ]; then
    printf 'Alpine Studio test discovery returned no tests.\n' >&2
    exit 1
fi

for test_name in $tests; do
    matches=0
    while IFS="$(printf '\t')" read -r prefix partition; do
        case "$test_name" in
            "$prefix"*) matches=$((matches + 1)) ;;
        esac
    done < "$manifest"
    if [ "$matches" -ne 1 ]; then
        printf 'Studio test has %s Miri partition matches: %s\n' "$matches" "$test_name" >&2
        exit 1
    fi
done

while IFS="$(printf '\t')" read -r prefix partition; do
    if ! printf '%s\n' "$tests" | awk -v prefix="$prefix" '
        index($0, prefix) == 1 { found = 1 }
        END { exit !found }
    '; then
        if [ "$(uname -s):$prefix" = 'Darwin:entry_point_contract_tests::' ]; then
            continue
        fi
        printf 'Studio Miri prefix matches no test: %s\n' "$prefix" >&2
        exit 1
    fi
done < "$manifest"

partitions=$(cut -f 2 "$manifest" | sort -u)
for partition in $partitions; do
    if ! awk -F '\t' -v partition="$partition" '$2 == partition { found = 1 } END { exit !found }' "$manifest"; then
        printf 'Studio Miri partition is empty: %s\n' "$partition" >&2
        exit 1
    fi
done

expected_partitions=$({
    printf '%s\n' foundation text text-layout
    printf '%s\n' "$partitions"
} | sort -u)
for workflow in .github/workflows/ci.yml .github/workflows/nightly-assurance.yml; do
    actual_partitions=$(sed -n 's/^[[:space:]]*- partition: //p' "$workflow" | sort)
    if [ "$actual_partitions" != "$expected_partitions" ]; then
        printf 'Miri workflow partition inventory differs from the manifest: %s\n' "$workflow" >&2
        exit 1
    fi
done

for invalid in '' unknown; do
    if scripts/run-miri-partition.sh "$invalid" >/dev/null 2>&1; then
        printf 'Miri runner accepted invalid partition: %s\n' "$invalid" >&2
        exit 1
    fi
done

printf 'Miri partition tests passed\n'
