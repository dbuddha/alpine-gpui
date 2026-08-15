#!/bin/sh
set -eu

output_dir=target/qualification
mkdir -p "$output_dir"

scripts/check-workload-hashes.sh

cargo run --quiet --locked -p alpine-assurance -- \
    validate-qualification assurance/qualification/v1/valid.toml \
    > "$output_dir/validation.txt"
cargo run --quiet --locked -p alpine-assurance -- \
    qualification-report assurance/qualification/v1/valid.toml \
    > "$output_dir/report.md"
cargo run --quiet --locked -p alpine-assurance -- \
    validate-scene-trace assurance/qualification/v1/scene.toml \
    > "$output_dir/scene-validation.txt"
cargo run --quiet --locked -p alpine-assurance -- \
    render-scene-reference assurance/qualification/v1/scene.toml \
    "$output_dir/reference.bgra" \
    > "$output_dir/reference-render.txt"

grep -Fq 'validated qualification renderer-foundation-fixture' \
    "$output_dir/validation.txt"
grep -Fq 'Comparison level: renderer-only' "$output_dir/report.md"
grep -Fq 'Environment qualified: true' "$output_dir/report.md"
grep -Fq 'with 3 operations and 8x4 reference pixels' \
    "$output_dir/scene-validation.txt"
test "$(wc -c < "$output_dir/reference.bgra" | tr -d ' ')" -eq 128

sed 's/alpha = 0.5/alpha = 0.25/' \
    assurance/qualification/v1/scene.toml \
    > "$output_dir/tampered-scene.toml"
if scripts/check-workload-hashes.sh \
    "$output_dir/tampered-scene.toml" > "$output_dir/tampered-hash.log" 2>&1; then
    printf 'tampered workload unexpectedly retained its declared identity\n' >&2
    exit 1
fi
grep -Fq 'workload hash mismatch' "$output_dir/tampered-hash.log"

assert_rejected() {
    fixture=$1
    expected=$2
    output="$output_dir/$(basename "$fixture" .toml).log"
    if cargo run --quiet --locked -p alpine-assurance -- \
        validate-qualification "$fixture" > "$output" 2>&1; then
        printf 'invalid qualification fixture unexpectedly passed: %s\n' \
            "$fixture" >&2
        exit 1
    fi
    grep -Fq "$expected" "$output"
}

assert_rejected \
    assurance/qualification/v1/mismatched-workload.toml \
    'workload hashes must match'
assert_rejected \
    assurance/qualification/v1/performance-before-correctness.toml \
    'equivalence gate visual did not pass'
assert_rejected \
    assurance/qualification/v1/unsupported-operation.toml \
    'unsupported scene operation silently-dropped-glyph'
assert_rejected \
    assurance/qualification/v1/unqualified-environment.toml \
    'performance measurement requires a qualified environment'

sed 's/independent_windows = 3/independent_windows = 2/' \
    assurance/qualification/v1/valid.toml \
    > "$output_dir/insufficient-reproduction.toml"
assert_rejected \
    "$output_dir/insufficient-reproduction.toml" \
    'requires three independent hardware windows'

printf 'golden qualification protocol checks passed\n'
