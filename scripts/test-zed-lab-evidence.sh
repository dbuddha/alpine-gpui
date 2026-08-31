#!/bin/sh
set -eu

output_dir=target/zed-lab-evidence
manifest=assurance/lab/v1/task-61-solid-quad.toml
manifest_v2=assurance/lab/v2/task-61-realistic-scenes.toml
manifest_v3=assurance/lab/v3/task-353-atlas-lifecycle.toml
mkdir -p "$output_dir"

assert_sha256() {
    expected=$1
    path=$2
    actual=$(shasum -a 256 "$path" | awk '{ print $1 }')
    [ "$actual" = "$expected" ] || {
        printf 'evidence hash mismatch for %s: expected %s, got %s\n' \
            "$path" "$expected" "$actual" >&2
        exit 1
    }
}

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

cargo run --quiet --locked -p alpine-assurance -- \
    validate-zed-lab-evidence "$manifest_v3" \
    > "$output_dir/validation-v3.txt"
cargo run --quiet --locked -p alpine-assurance -- \
    zed-lab-evidence-report "$manifest_v3" \
    > "$output_dir/report-v3.md"

grep -Fq 'across six transitions' "$output_dir/validation-v3.txt"
grep -Fq 'Compatible reuse | 0 B' "$output_dir/report-v3.md"
grep -Fq 'No timing, memory-superiority, latency, presentation, product, or performance claim is present' \
    "$output_dir/report-v3.md"

assert_sha256 1fb4eb35d0673a2f2451e450231328aa001f8cdda702954fc45f3a52580cb575 \
    assurance/lab/v3/source/hosted-qualification-set.toml
assert_sha256 db91c8a09c3a6bb405518bf58b30be0614bb2c596a6753b08c61317d0e037ff0 \
    assurance/lab/v3/source/hosted-atlas-lifecycle.toml
assert_sha256 65d0f161b82022c7686c04ee3729ed127b2fab2f80072868cbd20fa9922c571d \
    assurance/lab/v3/source/hosted-gpui-atlas-lifecycle.toml
assert_sha256 4181ba4e6785f29024370514515b14a1e82cf4dcce5de0c6664c3e08aed9a990 \
    assurance/lab/v3/source/physical-qualification-set.toml
assert_sha256 033016aa18b7bfe98834bb5ff9439b15f0b0567a004ca22252e8a6aee0381621 \
    assurance/lab/v3/source/physical-atlas-lifecycle.toml
assert_sha256 65d0f161b82022c7686c04ee3729ed127b2fab2f80072868cbd20fa9922c571d \
    assurance/lab/v3/source/physical-gpui-atlas-lifecycle.toml
assert_sha256 3a41f97947925be9940088fcb46fdaa4c1ac348bb149a715d8fbe922a8dbc231 \
    assurance/lab/v3/source/physical-alpine-atlas-lifecycle.toml
for digest in \
    fbb7e42e4eed8f8b98468fa42a339ffd615bfe81e4805046b599a5b9b8c1d4be \
    8f0c039370af535d9a9772207dd6db5ec169f8126d621d5f7ba96fe2afc63554; do
    readback="assurance/lab/v3/source/readbacks/$digest.bgra"
    assert_sha256 "$digest" "$readback"
    [ "$(wc -c < "$readback" | tr -d ' ')" -eq 256 ]
done

sed 's/performance_qualified = false/performance_qualified = true/' \
    "$manifest_v3" > "$output_dir/performance-claim-v3.toml"
if cargo run --quiet --locked -p alpine-assurance -- \
    validate-zed-lab-evidence "$output_dir/performance-claim-v3.toml" \
    > "$output_dir/performance-claim-v3.log" 2>&1; then
    printf 'unqualified lifecycle performance claim unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'must byte-match the reviewed canonical record' \
    "$output_dir/performance-claim-v3.log"

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
