#!/bin/sh
set -eu

root=target/native-benchmark-result-tests
rm -rf "$root"
mkdir -p "$root"
trap 'rm -rf "$root"' EXIT HUP INT TERM

stdout=$root/stdout
stderr=$root/stderr
output=$root/samples.csv
valid_stdout='recorded admission_iterations=1 warmup_iterations=1 sample_count=3 renderer=direct-metal trace=solid-quad at stage renderer-submit-readback using process-monotonic Instant; performance claim=none'
known_rejection='assurance error: cannot initialize Direct Metal: Metal device Apple Paravirtual device is unsupported: Metal 3 family support is required'

assert_rejected() {
    name=$1
    shift
    if scripts/check-native-benchmark-result.sh "$@" > "$root/$name.log" 2>&1; then
        printf 'invalid native benchmark result unexpectedly passed: %s\n' "$name" >&2
        exit 1
    fi
}

printf '%s\n' "$valid_stdout" > "$stdout"
: > "$stderr"
cat > "$output" <<'EOF'
sample_index,elapsed_ns
0,11
1,13
2,17
EOF
scripts/check-native-benchmark-result.sh 0 "$stdout" "$stderr" "$output" 3 >/dev/null

printf '%s\n' "$known_rejection" > "$stderr"
: > "$stdout"
rm "$output"
scripts/check-native-benchmark-result.sh 1 "$stdout" "$stderr" "$output" 3 >/dev/null
assert_rejected wrong-status 2 "$stdout" "$stderr" "$output" 3
printf 'unexpected\n' > "$stdout"
assert_rejected rejected-stdout 1 "$stdout" "$stderr" "$output" 3
: > "$stdout"
printf '%s\n' "$known_rejection" 'unrelated failure' > "$stderr"
assert_rejected mixed-failure 1 "$stdout" "$stderr" "$output" 3
printf '%s\n' 'assurance error: cannot initialize Direct Metal: Metal device Other is unsupported: Metal 3 family support is required' > "$stderr"
assert_rejected wrong-device 1 "$stdout" "$stderr" "$output" 3
printf '%s\n' 'assurance error: cannot initialize Direct Metal: Metal device Apple Paravirtual device is unsupported: family support is required' > "$stderr"
assert_rejected wrong-capability 1 "$stdout" "$stderr" "$output" 3
printf '%s\n' "$known_rejection" > "$stderr"
printf 'partial\n' > "$output"
assert_rejected rejected-output 1 "$stdout" "$stderr" "$output" 3

: > "$stderr"
printf '%s\n' "$valid_stdout" > "$stdout"
cat > "$output" <<'EOF'
sample_index,elapsed_ns
0,11
2,0
2,17
EOF
assert_rejected malformed-success 0 "$stdout" "$stderr" "$output" 3
sed 's/performance claim=none/performance claim=pending/' "$stdout" > "$root/no-claim"
assert_rejected missing-no-claim 0 "$root/no-claim" "$stderr" "$output" 3
sed 's/renderer-submit-readback/renderer-other/' "$stdout" > "$root/no-stage"
assert_rejected missing-stage 0 "$root/no-stage" "$stderr" "$output" 3
rm "$output"
assert_rejected missing-output 0 "$stdout" "$stderr" "$output" 3
assert_rejected zero-samples 0 "$stdout" "$stderr" "$output" 0

printf 'native benchmark result classifier tests passed\n'
