#!/bin/sh
set -eu

output_dir=target/zed-lab-evidence
manifest=assurance/lab/v1/task-61-solid-quad.toml
manifest_v2=assurance/lab/v2/task-61-realistic-scenes.toml
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

cargo run --quiet --locked -p alpine-assurance -- \
    validate-zed-lab-evidence "$manifest_v2" \
    > "$output_dir/validation-v2.txt"
cargo run --quiet --locked -p alpine-assurance -- \
    zed-lab-evidence-report "$manifest_v2" \
    > "$output_dir/report-v2.md"

grep -Fq 'across 8 fixtures' "$output_dir/validation-v2.txt"
grep -Fq 'No timing, memory, latency, presentation, product, or performance claim is present' \
    "$output_dir/report-v2.md"

sed 's/fixture_count = 8/fixture_count = 7/' \
    "$manifest_v2" > "$output_dir/wrong-fixture-count-v2.toml"
if cargo run --quiet --locked -p alpine-assurance -- \
    validate-zed-lab-evidence "$output_dir/wrong-fixture-count-v2.toml" \
    > "$output_dir/wrong-fixture-count-v2.log" 2>&1; then
    printf 'wrong version 2 fixture count unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'fixture_count must equal the eight-fixture trace ladder' \
    "$output_dir/wrong-fixture-count-v2.log"

sed 's/mutants_missed = 0/mutants_missed = 1/' \
    "$manifest_v2" > "$output_dir/missed-mutant-v2.toml"
if cargo run --quiet --locked -p alpine-assurance -- \
    validate-zed-lab-evidence "$output_dir/missed-mutant-v2.toml" \
    > "$output_dir/missed-mutant-v2.log" 2>&1; then
    printf 'missed version 2 mutant unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'mutation counts must classify every mutant' \
    "$output_dir/missed-mutant-v2.log"

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
