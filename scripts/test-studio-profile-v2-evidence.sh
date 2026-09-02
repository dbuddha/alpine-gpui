#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
checker=$repo_root/scripts/check-studio-profile-v2-evidence.sh
canonical=$repo_root/assurance/studio-profile/v2/544-frame-latency-negative
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

replace_value() {
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
gzip -dc "$hash_drift/two-frame/records.json.gz" > "$scratch/hash-drift.json"
gzip -n -1 -c "$scratch/hash-drift.json" > "$hash_drift/two-frame/records.json.gz"
expect_failure hash-drift 'two-frame normalized gzip hash mismatch' "$hash_drift"

privacy_drift=$scratch/privacy-drift
cp -R "$canonical" "$privacy_drift"
gzip -dc "$privacy_drift/one-frame/records.json.gz" > "$scratch/privacy.json"
jq -S -c 'map(.processImagePath = "/Users/private/Alpine Studio")' \
    "$scratch/privacy.json" > "$scratch/privacy-next.json"
gzip -n -9 -c "$scratch/privacy-next.json" > "$privacy_drift/one-frame/records.json.gz"
replace_value "$privacy_drift/manifest.toml" one_frame_normalized_records_sha256 \
    "\"$(hash_file "$scratch/privacy-next.json")\""
replace_value "$privacy_drift/manifest.toml" one_frame_normalized_records_gzip_sha256 \
    "\"$(hash_file "$privacy_drift/one-frame/records.json.gz")\""
replace_value "$privacy_drift/manifest.toml" one_frame_normalized_records_bytes \
    "$(wc -c < "$scratch/privacy-next.json" | tr -d ' ')"
expect_failure privacy-drift \
    'one-frame normalized records failed privacy and shape contract' \
    "$privacy_drift"

derived_drift=$scratch/derived-drift
cp -R "$canonical" "$derived_drift"
printf '999999\t33333250\t0\n' >> \
    "$derived_drift/two-frame/analysis/presentation-derived.tsv"
replace_value "$derived_drift/manifest.toml" two_frame_presentation_derived_sha256 \
    "\"$(hash_file "$derived_drift/two-frame/analysis/presentation-derived.tsv")\""
expect_failure derived-drift \
    'two-frame rederived presentation output drift' \
    "$derived_drift"

claim_drift=$scratch/claim-drift
cp -R "$canonical" "$claim_drift"
replace_value "$claim_drift/manifest.toml" causal_attribution_allowed true
expect_failure claim-drift \
    'causal_attribution_allowed must remain false' \
    "$claim_drift"

fabricated_improvement=$scratch/fabricated-improvement
cp -R "$canonical" "$fabricated_improvement"
awk -F '\t' 'BEGIN { OFS="\t" }
    $1 == "one-frame" {
        $3=16666625; $4=16666625; $5=16666625; $6=16666625; $7=16666625
    }
    { print }
' "$fabricated_improvement/pair-summary.tsv" > \
    "$fabricated_improvement/pair-summary.tsv.next"
mv "$fabricated_improvement/pair-summary.tsv.next" \
    "$fabricated_improvement/pair-summary.tsv"
replace_value "$fabricated_improvement/manifest.toml" pair_summary_sha256 \
    "\"$(hash_file "$fabricated_improvement/pair-summary.tsv")\""
expect_failure fabricated-improvement \
    'one-frame pair summary changed' \
    "$fabricated_improvement"

printf 'retained Studio frame-latency v2 evidence tests passed\n'
