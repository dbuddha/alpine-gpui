#!/bin/sh
set -eu

output_dir=target/studio-residency-contract
mkdir -p "$output_dir"

write_fixture() {
    path=$1
    growth=$2
    cat > "$path" <<EOF
{
  "unit": "byte",
  "bytes per unit": 1,
  "samples": [
    {"start_time":{"wall_time_s":1000.0},"processes":[{"pid":42,"auxiliary":{"phys_footprint":1000000,"phys_footprint_peak":1000000}}],"summary":{"total":{"dirty":800000}}},
    {"start_time":{"wall_time_s":1001.0},"processes":[{"pid":42,"auxiliary":{"phys_footprint":$((1000000 + growth)),"phys_footprint_peak":$((1000000 + growth))}}],"summary":{"total":{"dirty":$((800000 + growth))}}},
    {"start_time":{"wall_time_s":1002.0},"processes":[{"pid":42,"auxiliary":{"phys_footprint":$((1000000 + growth * 2)),"phys_footprint_peak":$((1000000 + growth * 2))}}],"summary":{"total":{"dirty":$((800000 + growth * 2))}}},
    {"start_time":{"wall_time_s":1003.0},"processes":[{"pid":42,"auxiliary":{"phys_footprint":$((1000000 + growth * 3)),"phys_footprint_peak":$((1000000 + growth * 3))}}],"summary":{"total":{"dirty":$((800000 + growth * 3))}}},
    {"start_time":{"wall_time_s":1004.0},"processes":[{"pid":42,"auxiliary":{"phys_footprint":$((1000000 + growth * 4)),"phys_footprint_peak":$((1000000 + growth * 4))}}],"summary":{"total":{"dirty":$((800000 + growth * 4))}}}
  ]
}
EOF
}

write_fixture "$output_dir/stable.json" 0
write_fixture "$output_dir/growing.json" 4096

scripts/analyze-studio-residency.sh "$output_dir/stable.json" 42 1 \
    "$output_dir/stable" 1024 > "$output_dir/stable.log"
grep -Fq 'sample_count = 5' "$output_dir/stable/summary.toml"
grep -Fq 'warm_sample_count = 4' "$output_dir/stable/summary.toml"
grep -Fq 'physical_slope_bytes_per_second = 0.000000' \
    "$output_dir/stable/summary.toml"
grep -Fq 'window_status = "pass"' "$output_dir/stable/summary.toml"

if scripts/analyze-studio-residency.sh "$output_dir/growing.json" 42 1 \
    "$output_dir/growing" 1024 > "$output_dir/growing.log" 2>&1; then
    printf 'growing residency fixture unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'physical_slope_bytes_per_second = 4096.000000' \
    "$output_dir/growing/summary.toml"
grep -Fq 'window_status = "fail"' "$output_dir/growing/summary.toml"

if scripts/analyze-studio-residency.sh "$output_dir/stable.json" 99 1 \
    "$output_dir/wrong-pid" > "$output_dir/wrong-pid.log" 2>&1; then
    printf 'mismatched process identity unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'belongs to PID 42, expected 99' "$output_dir/wrong-pid.log"

sed 's/"unit": "byte"/"unit": "page"/' "$output_dir/stable.json" \
    > "$output_dir/wrong-unit.json"
if scripts/analyze-studio-residency.sh "$output_dir/wrong-unit.json" 42 1 \
    "$output_dir/wrong-unit" > "$output_dir/wrong-unit.log" 2>&1; then
    printf 'non-byte footprint fixture unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'must use byte units' "$output_dir/wrong-unit.log"

if scripts/capture-studio-residency.sh --help | grep -Fq \
    'capture-studio-residency.sh'; then
    :
else
    printf 'capture usage is unavailable\n' >&2
    exit 1
fi

printf 'Studio residency contract checks passed\n'
