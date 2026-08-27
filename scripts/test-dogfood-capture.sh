#!/bin/sh
set -eu

output_dir=target/dogfood-capture
bundle_dir="$output_dir/bundle"
recorded_dir="$output_dir/recorded"
mkdir -p "$bundle_dir"
rm -rf "$recorded_dir"

for artifact in \
    assurance/dogfood/v1/session.toml \
    assurance/dogfood/v1/snapshot.toml; do
    attributes=$(git check-attr text eol -- "$artifact")
    printf '%s\n' "$attributes" | grep -Fq "$artifact: text: set"
    printf '%s\n' "$attributes" | grep -Fq "$artifact: eol: lf"
done

cargo run --quiet --locked -p alpine-assurance -- \
    validate-studio-dogfood assurance/dogfood/v1/session.toml \
    > "$output_dir/validation.txt"
cargo run --quiet --locked -p alpine-assurance -- \
    studio-dogfood-report assurance/dogfood/v1/session.toml \
    > "$output_dir/report.md"

grep -Fq 'validated Studio dogfood capture fixture-dogfood-session' \
    "$output_dir/validation.txt"
grep -Fq 'Performance claim: none' "$output_dir/report.md"
grep -Fq 'Idle submissions: 0' "$output_dir/report.md"
grep -Fq '`glyph-atlas-gpu`' "$output_dir/report.md"

cargo run --quiet --locked -p alpine-assurance -- \
    record-studio-dogfood \
    assurance/dogfood/v1/session.toml \
    assurance/dogfood/v1/snapshot.toml \
    "$recorded_dir" > "$output_dir/recorded.txt"
grep -Fq 'recorded Studio dogfood capture fixture-dogfood-session' \
    "$output_dir/recorded.txt"
cargo run --quiet --locked -p alpine-assurance -- \
    validate-studio-dogfood "$recorded_dir/session.toml" \
    > "$output_dir/recorded-validation.txt"
if cargo run --quiet --locked -p alpine-assurance -- \
    record-studio-dogfood \
    assurance/dogfood/v1/session.toml \
    assurance/dogfood/v1/snapshot.toml \
    "$recorded_dir" > "$output_dir/recorded-overwrite.log" 2>&1; then
    printf 'dogfood recorder unexpectedly overwrote an existing bundle\n' >&2
    exit 1
fi
grep -Fq 'already exists' "$output_dir/recorded-overwrite.log"

cp assurance/dogfood/v1/session.toml "$bundle_dir/session.toml"
cp assurance/dogfood/v1/snapshot.toml "$bundle_dir/snapshot.toml"
sed -i.bak 's/fixture capture completed/tampered capture/' \
    "$bundle_dir/snapshot.toml"
rm "$bundle_dir/snapshot.toml.bak"
if cargo run --quiet --locked -p alpine-assurance -- \
    validate-studio-dogfood "$bundle_dir/session.toml" \
    > "$output_dir/tampered-snapshot.log" 2>&1; then
    printf 'tampered dogfood snapshot unexpectedly validated\n' >&2
    exit 1
fi
grep -Fq 'snapshot SHA-256 mismatch' "$output_dir/tampered-snapshot.log"

cp assurance/dogfood/v1/snapshot.toml "$bundle_dir/snapshot.toml"
sed 's/telemetry = false/telemetry = true/' \
    assurance/dogfood/v1/session.toml > "$bundle_dir/session.toml"
if cargo run --quiet --locked -p alpine-assurance -- \
    validate-studio-dogfood "$bundle_dir/session.toml" \
    > "$output_dir/telemetry.log" 2>&1; then
    printf 'telemetry-enabled dogfood capture unexpectedly validated\n' >&2
    exit 1
fi
grep -Fq 'capture must disable telemetry' "$output_dir/telemetry.log"

printf 'bounded Studio dogfood capture checks passed\n'
