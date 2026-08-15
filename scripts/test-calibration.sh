#!/bin/sh
set -eu

output_dir=target/calibration
mkdir -p "$output_dir"

cargo run --quiet --locked -p alpine-assurance -- \
    validate-aa-calibration assurance/calibration/v1/valid.toml \
    > "$output_dir/validation.txt"
cargo run --quiet --locked -p alpine-assurance -- \
    aa-calibration-report assurance/calibration/v1/valid.toml \
    > "$output_dir/report.md"

grep -Fq '20 runs, 4 windows, and 40 pairs' "$output_dir/validation.txt"
grep -Fq 'no performance claim' "$output_dir/validation.txt"
grep -Fq 'Status: fixture-only' "$output_dir/report.md"
grep -Fq 'Performance claim: none' "$output_dir/report.md"
grep -Fq 'Measurement stage: renderer-submit-readback' "$output_dir/report.md"
grep -Fq 'Sample class: warm' "$output_dir/report.md"
grep -Fq 'Warmup iterations: 10' "$output_dir/report.md"

assert_rejected() {
    fixture=$1
    expected=$2
    output="$output_dir/$(basename "$fixture" .toml).log"
    if cargo run --quiet --locked -p alpine-assurance -- \
        validate-aa-calibration "$fixture" > "$output" 2>&1; then
        printf 'invalid calibration fixture unexpectedly passed: %s\n' \
            "$fixture" >&2
        exit 1
    fi
    grep -Fq "$expected" "$output"
}

sed \
    's/candidate_revision = "b567e8f29c3c6c6bcdf98c02bc1958e59f044157"/candidate_revision = "2222222222222222222222222222222222222222"/' \
    assurance/calibration/v1/valid.toml \
    > "$output_dir/revision-mismatch.toml"
assert_rejected \
    "$output_dir/revision-mismatch.toml" \
    'A/A base and candidate revisions must match'

sed \
    's|raw_samples_artifact = "assurance/calibration/v1/raw/aa-samples.csv"|raw_samples_artifact = "../aa-samples.csv"|' \
    assurance/calibration/v1/valid.toml \
    > "$output_dir/unsafe-artifact.toml"
assert_rejected \
    "$output_dir/unsafe-artifact.toml" \
    'repository-relative normal path'

sed \
    's/raw_samples_sha256 = "694f/raw_samples_sha256 = "0000/' \
    assurance/calibration/v1/valid.toml \
    > "$output_dir/hash-mismatch.toml"
assert_rejected \
    "$output_dir/hash-mismatch.toml" \
    'raw sample artifact SHA-256 does not match'

sed \
    's/environment_kind = "test-fixture"/environment_kind = "hosted-virtual"/' \
    assurance/calibration/v1/valid.toml \
    > "$output_dir/hosted-virtual.toml"
assert_rejected \
    "$output_dir/hosted-virtual.toml" \
    'not qualified physical or test-fixture evidence'

sed \
    's/sample_class = "warm"/sample_class = "cold"/' \
    assurance/calibration/v1/valid.toml \
    > "$output_dir/cold-with-warmup.toml"
assert_rejected \
    "$output_dir/cold-with-warmup.toml" \
    'cold calibration requires zero warmup iterations'

sed \
    's/ended_at_utc = "2026-08-01T11:00:00Z"/ended_at_utc = "2026-08-01T09:00:00Z"/' \
    assurance/calibration/v1/valid.toml \
    > "$output_dir/reversed-window.toml"
assert_rejected \
    "$output_dir/reversed-window.toml" \
    'ordered second-resolution UTC timestamps'

awk '
    /^\[\[runs\]\]$/ { run_count += 1 }
    run_count == 20 { exit }
    { print }
' assurance/calibration/v1/valid.toml \
    > "$output_dir/insufficient-runs.toml"
assert_rejected \
    "$output_dir/insufficient-runs.toml" \
    'requires at least 20 runs'

cp assurance/calibration/v1/valid.toml "$output_dir/unknown-field.toml"
printf 'unreviewed = true\n' >> "$output_dir/unknown-field.toml"
assert_rejected \
    "$output_dir/unknown-field.toml" \
    'unknown field'

printf 'renderer A/A calibration protocol checks passed\n'
