#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
checker=$repo_root/scripts/check-studio-profile-v2-evidence.sh
canonical=$repo_root/assurance/studio-profile/v2/553-command-buffer-presentation
scratch=$(mktemp -d "${TMPDIR:-/tmp}/alpine-profile-v2-evidence-test.XXXXXX")
cleanup() { rm -rf "$scratch"; }
trap cleanup EXIT HUP INT TERM

hash_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

replace_string() {
    file=$1
    key=$2
    value=$3
    output=$file.next
    awk -v key="$key" -v value="$value" '
        $1 == key && $2 == "=" { print key " = \"" value "\""; found = 1; next }
        { print }
        END { if (!found) exit 1 }
    ' "$file" > "$output"
    mv "$output" "$file"
}

replace_integer() {
    file=$1
    key=$2
    value=$3
    output=$file.next
    awk -v key="$key" -v value="$value" '
        $1 == key && $2 == "=" { print key " = " value; found = 1; next }
        { print }
        END { if (!found) exit 1 }
    ' "$file" > "$output"
    mv "$output" "$file"
}

expect_failure() {
    name=$1
    expected=$2
    package=$3
    log=$scratch/$name.log
    if "$checker" "$package" >"$log" 2>&1; then
        printf 'invalid retained v2 profile unexpectedly passed: %s\n' "$name" >&2
        exit 1
    fi
    grep -Fq "$expected" "$log"
}

"$checker" "$canonical"

hash_drift=$scratch/hash-drift
cp -R "$canonical" "$hash_drift"
printf 'x' >> "$hash_drift/candidate/records.json.gz"
expect_failure hash-drift 'candidate normalized gzip hash mismatch' "$hash_drift"

derived_drift=$scratch/derived-drift
cp -R "$canonical" "$derived_drift"
printf 'bogus\t1\t1\t1\t1\t1\t1\n' >> "$derived_drift/candidate/analysis/summary.tsv"
replace_string "$derived_drift/manifest.toml" candidate_analysis_summary_sha256 \
    "$(hash_file "$derived_drift/candidate/analysis/summary.tsv")"
expect_failure derived-drift 'candidate reanalysis output drift: summary.tsv' "$derived_drift"

privacy_drift=$scratch/privacy-drift
cp -R "$canonical" "$privacy_drift"
gzip -dc "$privacy_drift/candidate/records.json.gz" > "$scratch/privacy.json"
jq 'map(.processImagePath = "/Users/private/Alpine Studio")' \
    "$scratch/privacy.json" > "$scratch/privacy-next.json"
gzip -n -9 -c "$scratch/privacy-next.json" > "$privacy_drift/candidate/records.json.gz"
replace_string "$privacy_drift/manifest.toml" candidate_normalized_records_sha256 \
    "$(hash_file "$scratch/privacy-next.json")"
replace_string "$privacy_drift/manifest.toml" candidate_normalized_records_gzip_sha256 \
    "$(hash_file "$privacy_drift/candidate/records.json.gz")"
replace_integer "$privacy_drift/manifest.toml" candidate_normalized_records_bytes \
    "$(wc -c < "$scratch/privacy-next.json" | tr -d ' ')"
expect_failure privacy-drift \
    'candidate normalized records failed privacy and shape contract' "$privacy_drift"

comparison_drift=$scratch/comparison-drift
cp -R "$canonical" "$comparison_drift"
printf 'invented\t1\t1\t1\t1\t0\n' >> "$comparison_drift/comparison.tsv"
replace_string "$comparison_drift/manifest.toml" comparison_sha256 \
    "$(hash_file "$comparison_drift/comparison.tsv")"
expect_failure comparison-drift 'derived comparison drifted' "$comparison_drift"

printf 'retained Studio profile v2 evidence tests passed\n'
