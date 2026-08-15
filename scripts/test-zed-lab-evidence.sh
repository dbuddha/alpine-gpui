#!/bin/sh
set -eu

output_dir=target/zed-lab-evidence
manifest=assurance/lab/v1/task-61-solid-quad.toml
mkdir -p "$output_dir"

cargo run --quiet --locked -p alpine-assurance -- \
    validate-zed-lab-evidence "$manifest" \
    > "$output_dir/validation.txt"
cargo run --quiet --locked -p alpine-assurance -- \
    zed-lab-evidence-report "$manifest" \
    > "$output_dir/report.md"

grep -Fq 'task #61 with hosted offline GPUI and physical Direct Metal' \
    "$output_dir/validation.txt"
grep -Fq 'No timing or performance claim is present' "$output_dir/report.md"

awk '
    /^\[physical\]$/ { physical = 1 }
    physical && /^alpine_metal_sha256 = / {
        print "alpine_metal_sha256 = \"0000000000000000000000000000000000000000000000000000000000000000\""
        next
    }
    { print }
' "$manifest" > "$output_dir/divergent-physical.toml"
if cargo run --quiet --locked -p alpine-assurance -- \
    validate-zed-lab-evidence "$output_dir/divergent-physical.toml" \
    > "$output_dir/divergent-physical.log" 2>&1; then
    printf 'divergent physical evidence unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'physical Direct Metal hash must match' \
    "$output_dir/divergent-physical.log"

sed 's/performance_qualified = false/performance_qualified = true/' \
    "$manifest" > "$output_dir/performance-claim.toml"
if cargo run --quiet --locked -p alpine-assurance -- \
    validate-zed-lab-evidence "$output_dir/performance-claim.toml" \
    > "$output_dir/performance-claim.log" 2>&1; then
    printf 'unqualified performance claim unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'cannot contain a performance claim' \
    "$output_dir/performance-claim.log"

sed 's/retention_days = 90/retention_days = 7/' \
    "$manifest" > "$output_dir/short-retention.toml"
if cargo run --quiet --locked -p alpine-assurance -- \
    validate-zed-lab-evidence "$output_dir/short-retention.toml" \
    > "$output_dir/short-retention.log" 2>&1; then
    printf 'short evidence retention unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'retained for exactly 90 days' "$output_dir/short-retention.log"

printf 'Zed lab evidence checks passed\n'
