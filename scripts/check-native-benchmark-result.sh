#!/bin/sh
set -eu

fail() {
    printf 'native benchmark result error: %s\n' "$*" >&2
    exit 1
}

[ "$#" -eq 5 ] || fail 'expected status, stdout, stderr, output, and sample count'
status=$1
stdout=$2
stderr=$3
output=$4
sample_count=$5

case "$status" in
    0|1) ;;
    *) fail 'status must be zero or the exact known rejection status one' ;;
esac
case "$sample_count" in
    ''|*[!0-9]*) fail 'sample count must be a positive integer' ;;
    0) fail 'sample count must be a positive integer' ;;
esac
for path in "$stdout" "$stderr"; do
    [ -f "$path" ] || fail "missing result stream: $path"
    [ ! -L "$path" ] || fail "result stream must not be a symbolic link: $path"
done

if [ "$status" -eq 0 ]; then
    [ -f "$output" ] || fail 'successful sampler did not publish output'
    [ ! -L "$output" ] || fail 'sampler output must not be a symbolic link'
    grep -Fq 'stage renderer-submit-readback using process-monotonic Instant' "$stdout" ||
        fail 'successful sampler omitted the measured stage'
    grep -Fq 'performance claim=none' "$stdout" ||
        fail 'successful sampler omitted the no-claim disclosure'
    awk -F, -v expected="$sample_count" '
        NR == 1 {
            if ($1 != "sample_index" || $2 != "elapsed_ns" || NF != 2) exit 1
            next
        }
        $1 != NR - 2 || $2 !~ /^[1-9][0-9]*$/ || NF != 2 { exit 1 }
        END { if (NR != expected + 1) exit 1 }
    ' "$output" || fail 'successful sampler output is malformed or incomplete'
    printf 'native renderer sampling completed with %s no-claim samples\n' "$sample_count"
    exit 0
fi

[ ! -e "$output" ] || fail 'rejected sampler published output'
[ ! -s "$stdout" ] || fail 'rejected sampler published success output'
known_rejection='assurance error: cannot initialize Direct Metal: Metal device Apple Paravirtual device is unsupported: Metal 3 family support is required'
[ "$(grep -Fxc "$known_rejection" "$stderr" || true)" -eq 1 ] ||
    fail 'sampler failure is not the exact known paravirtual capability rejection'
awk -v known="$known_rejection" '
    $0 == known { next }
    /Metal API Validation Enabled$/ { next }
    /Metal GPU Validation Enabled$/ { next }
    NF == 0 { next }
    { exit 1 }
' "$stderr" || fail 'paravirtual rejection included an unrelated error'
printf '%s\n' 'native renderer sampling not-run on known Apple Paravirtual device; performance claim=none'
