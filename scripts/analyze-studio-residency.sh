#!/bin/sh
set -eu

usage() {
    cat <<'EOF'
usage: analyze-studio-residency.sh RAW_JSON EXPECTED_PID WARMUP_SECONDS OUTPUT_DIR [SLOPE_LIMIT_BYTES_PER_SECOND]
EOF
}

fail() {
    printf 'studio residency analysis failed: %s\n' "$1" >&2
    exit 1
}

is_uint() {
    case $1 in
        '' | *[!0-9]*) return 1 ;;
        *) return 0 ;;
    esac
}

[ "$#" -ge 4 ] && [ "$#" -le 5 ] || {
    usage >&2
    exit 2
}

raw_json=$1
expected_pid=$2
warmup_seconds=$3
output_dir=$4
slope_limit=${5-}

[ -f "$raw_json" ] || fail "raw footprint JSON is missing"
is_uint "$expected_pid" || fail "expected PID must be an unsigned integer"
is_uint "$warmup_seconds" || fail "warmup must be whole seconds"
if [ -n "$slope_limit" ]; then
    is_uint "$slope_limit" || fail "slope limit must be unsigned bytes per second"
fi

unit=$(/usr/bin/plutil -extract unit raw -o - "$raw_json" 2>/dev/null) ||
    fail "raw footprint JSON is invalid or its unit is missing"
bytes_per_unit=$(/usr/bin/plutil -extract 'bytes per unit' raw -o - \
    "$raw_json" 2>/dev/null) || fail "raw footprint byte scale is missing"
[ "$unit" = byte ] || fail "raw footprint must use byte units"
[ "$bytes_per_unit" = 1 ] || fail "raw footprint must use one byte per unit"

mkdir -p "$output_dir"
raw_samples="$output_dir/raw-samples.tsv"
samples_csv="$output_dir/samples.csv"
summary="$output_dir/summary.toml"
: > "$raw_samples"

index=0
while /usr/bin/plutil -extract "samples.$index" xml1 -o /dev/null \
    "$raw_json" >/dev/null 2>&1; do
    extract() {
        /usr/bin/plutil -extract "samples.$index.$1" raw -o - \
            "$raw_json" 2>/dev/null
    }

    pid=$(extract processes.0.pid) || fail "sample $index has no process PID"
    wall_time=$(extract start_time.wall_time_s) ||
        fail "sample $index has no monotonic wall identity"
    physical=$(extract processes.0.auxiliary.phys_footprint) ||
        fail "sample $index has no physical footprint"
    physical_peak=$(extract processes.0.auxiliary.phys_footprint_peak) ||
        fail "sample $index has no physical footprint peak"
    private_dirty=$(extract summary.total.dirty) ||
        fail "sample $index has no private dirty total"

    [ "$pid" = "$expected_pid" ] ||
        fail "sample $index belongs to PID $pid, expected $expected_pid"
    for value in "$physical" "$physical_peak" "$private_dirty"; do
        is_uint "$value" || fail "sample $index contains a non-integer byte value"
    done
    if /usr/bin/plutil -extract "samples.$index.processes.1.pid" raw \
        -o /dev/null "$raw_json" >/dev/null 2>&1; then
        fail "sample $index contains more than one process"
    fi

    printf '%s\t%s\t%s\t%s\t%s\n' \
        "$index" "$wall_time" "$physical" "$physical_peak" "$private_dirty" \
        >> "$raw_samples"
    index=$((index + 1))
done

[ "$index" -ge 4 ] || fail "at least four physical samples are required"
[ "$index" -le 4096 ] || fail "sample count exceeds the 4096-sample evidence bound"

awk -F '\t' 'BEGIN {
    OFS=",";
    print "sample_index,wall_time_s,elapsed_s,physical_footprint_bytes,physical_peak_bytes,private_dirty_bytes"
}
NR == 1 { first = $2 }
{
    printf "%s,%s,%.6f,%s,%s,%s\n", $1, $2, $2 - first, $3, $4, $5
}' "$raw_samples" > "$samples_csv"

stats=$(awk -F ',' -v warmup="$warmup_seconds" '
NR == 1 { next }
{
    count++;
    current_physical = $4;
    current_dirty = $6;
    if ($4 > peak_physical) peak_physical = $4;
    if ($5 > reported_peak) reported_peak = $5;
    if ($6 > peak_dirty) peak_dirty = $6;
    if ($3 >= warmup) {
        n++;
        x = $3;
        sx += x;
        sx2 += x * x;
        sy_physical += $4;
        sxy_physical += x * $4;
        sy_dirty += $6;
        sxy_dirty += x * $6;
    }
}
END {
    denominator = n * sx2 - sx * sx;
    if (n < 3 || denominator <= 0) exit 3;
    physical_slope = (n * sxy_physical - sx * sy_physical) / denominator;
    dirty_slope = (n * sxy_dirty - sx * sy_dirty) / denominator;
    printf "%d %d %.6f %.6f %.0f %.0f %.0f %.0f\n", \
        count, n, physical_slope, dirty_slope, current_physical, \
        peak_physical, reported_peak, peak_dirty;
}' "$samples_csv") || fail "warm window needs three distinct samples"

set -- $stats
sample_count=$1
warm_sample_count=$2
physical_slope=$3
dirty_slope=$4
current_physical=$5
peak_physical=$6
reported_peak=$7
peak_dirty=$8
raw_sha=$(/usr/bin/shasum -a 256 "$raw_json" | awk '{print $1}')

window_status=informational
exit_status=0
if [ -n "$slope_limit" ]; then
    if awk -v physical="$physical_slope" -v dirty="$dirty_slope" \
        -v limit="$slope_limit" \
        'BEGIN { exit !((physical > limit) || (dirty > limit)) }'; then
        window_status=fail
        exit_status=1
    else
        window_status=pass
    fi
fi

cat > "$summary" <<EOF
schema = "alpine-studio-residency-analysis/v1"
raw_sha256 = "$raw_sha"
pid = $expected_pid
sample_count = $sample_count
warm_sample_count = $warm_sample_count
warmup_seconds = $warmup_seconds
current_physical_footprint_bytes = $current_physical
peak_sampled_physical_footprint_bytes = $peak_physical
reported_peak_physical_footprint_bytes = $reported_peak
peak_private_dirty_bytes = $peak_dirty
physical_slope_bytes_per_second = $physical_slope
private_dirty_slope_bytes_per_second = $dirty_slope
slope_limit_bytes_per_second = "${slope_limit:-not-calibrated}"
window_status = "$window_status"
EOF

printf 'analyzed %s samples; window status: %s\n' \
    "$sample_count" "$window_status"
exit "$exit_status"
