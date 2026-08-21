#!/bin/sh
set -eu

output_dir=target/dogfood-capture
bundle_dir="$output_dir/bundle"
mkdir -p "$bundle_dir"

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
