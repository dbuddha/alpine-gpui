#!/bin/sh
set -eu

root=target/studio-ax-process-capture-contract
rm -rf "$root"
mkdir -p "$root"

cat > "$root/fake-studio" <<'EOF'
#!/bin/sh
sleep 2
exit 0
EOF

cat > "$root/fake-assurance" <<'EOF'
#!/bin/sh
[ "$#" -eq 6 ] || exit 20
[ "$1" = capture-ax-client ] || exit 21
[ "$2" -gt 0 ] || exit 22
[ "$3" -gt 0 ] || exit 23
output=$6
mkdir "$output"
printf '%s\n' '{"sequence":1,"depth":0}' > "$output/tree.jsonl"
printf '%s\n' '{"sequence":1,"kind":"focus"}' > "$output/events.jsonl"
printf '%s\n' '{"sequence":1,"operation":"query"}' > "$output/latency.jsonl"
printf 'fixture raw AX capture\n'
EOF

cat > "$root/fake-no-output" <<'EOF'
#!/bin/sh
exit 0
EOF

cat > "$root/fake-untrusted" <<'EOF'
#!/bin/sh
printf 'assurance error: AX client is not trusted\n' >&2
exit 32
EOF

cat > "$root/fake-footprint" <<'EOF'
#!/bin/sh
pid=
output=
while [ "$#" -gt 0 ]; do
    case $1 in
        --pid) pid=$2; shift 2 ;;
        --json) output=$2; shift 2 ;;
        --sample|--sample-duration|--format) shift 2 ;;
        --noCategories) shift ;;
        *) exit 30 ;;
    esac
done
cat > "$output" <<JSON
{"unit":"byte","bytes per unit":1,"samples":[
{"start_time":{"wall_time_s":1000.0},"processes":[{"pid":$pid,"auxiliary":{"phys_footprint":100,"phys_footprint_peak":100}}],"summary":{"total":{"dirty":50}}},
{"start_time":{"wall_time_s":1001.0},"processes":[{"pid":$pid,"auxiliary":{"phys_footprint":110,"phys_footprint_peak":110}}],"summary":{"total":{"dirty":55}}},
{"start_time":{"wall_time_s":1002.0},"processes":[{"pid":$pid,"auxiliary":{"phys_footprint":105,"phys_footprint_peak":110}}],"summary":{"total":{"dirty":52}}},
{"start_time":{"wall_time_s":1003.0},"processes":[{"pid":$pid,"auxiliary":{"phys_footprint":108,"phys_footprint_peak":110}}],"summary":{"total":{"dirty":54}}}
]}
JSON
EOF

cat > "$root/fake-sampler-failure" <<'EOF'
#!/bin/sh
exit 31
EOF

cat > "$root/fake-studio-failure" <<'EOF'
#!/bin/sh
sleep 1
exit 7
EOF

cat > "$root/fake-partial-failure" <<'EOF'
#!/bin/sh
mkdir "$6"
printf '%s\n' '{"fixture_only":true,"event":"before-failure"}' > "$6/events.jsonl"
printf 'injected failure after raw output\n' >&2
exit 17
EOF

cat > "$root/fake-large-failure" <<'EOF'
#!/bin/sh
mkdir "$6"
for name in tree.jsonl events.jsonl latency.jsonl; do
    /usr/bin/head -c 131072 /dev/zero > "$6/$name"
done
exit 17
EOF

cat > "$root/fake-symlink-failure" <<'EOF'
#!/bin/sh
mkdir "$6"
ln -s "$AX_TEST_EXTERNAL_FILE" "$6/events.jsonl"
exit 17
EOF

cat > "$root/fake-parent-symlink-failure" <<'EOF'
#!/bin/sh
ln -s "$AX_TEST_EXTERNAL_DIRECTORY" "$6"
exit 17
EOF

cat > "$root/fake-studio-timeout" <<'EOF'
#!/bin/sh
exec sleep 20
EOF

cat > "$root/fake-late-collision" <<'EOF'
#!/bin/sh
mkdir "$AX_TEST_DESTINATION"
printf 'preserve existing output\n' > "$AX_TEST_DESTINATION/sentinel"
exec "$AX_TEST_GOOD_CLIENT" "$@"
EOF

chmod +x "$root/fake-studio" "$root/fake-assurance" \
    "$root/fake-no-output" "$root/fake-untrusted" "$root/fake-footprint" \
    "$root/fake-sampler-failure" "$root/fake-studio-failure" \
    "$root/fake-partial-failure" "$root/fake-large-failure" \
    "$root/fake-symlink-failure" "$root/fake-parent-symlink-failure" \
    "$root/fake-studio-timeout" "$root/fake-late-collision"

capture_script=${AX_TEST_CAPTURE_SCRIPT-scripts/capture-studio-ax-process.sh}

capture() {
    binary=$1
    assurance=$2
    sampler=$3
    destination=$4
    "$capture_script" \
        --binary "$binary" --assurance "$assurance" \
        --repository . --workspace . --output-dir "$destination" \
        --generation 1 --pre-action-ms 1 --post-action-ms 1 \
        --duration-seconds 3 --interval-seconds 1 \
        --post-close-timeout-seconds "${AX_TEST_CLOSE_TIMEOUT-5}" --opt-in --fixture-only \
        --sampler "$sampler"
}

capture "$root/fake-studio" "$root/fake-assurance" \
    "$root/fake-footprint" "$root/package" > "$root/capture.log"

grep -Fq 'schema = "alpine-ax-process-input/v1"' "$root/package/manifest.toml"
grep -Fq 'task_issue = 504' "$root/package/manifest.toml"
grep -Fq 'fixture_only = true' "$root/package/manifest.toml"
grep -Fq 'performance_claim = false' "$root/package/manifest.toml"
grep -Fq 'aep_0273_bundle_ready = false' "$root/package/manifest.toml"
grep -Fq 'post_close_absent = true' "$root/package/manifest.toml"
grep -Fq 'sample_count = 4' "$root/package/residency-analysis/summary.toml"
for artifact in raw-ax/tree.jsonl raw-ax/events.jsonl raw-ax/latency.jsonl \
    footprint.json residency-analysis/samples.csv residency-analysis/summary.toml \
    studio.stdout studio.stderr ax.stdout ax.stderr workspace-record.txt; do
    test -f "$root/package/$artifact"
done
tree_sha=$(/usr/bin/shasum -a 256 "$root/package/raw-ax/tree.jsonl" | awk '{print $1}')
grep -Fq "sha256 = \"$tree_sha\"" "$root/package/manifest.toml"
test ! -e "$root/package/rejection.txt"

if capture "$root/fake-studio" "$root/fake-assurance" \
    "$root/fake-footprint" "$root/package" > "$root/overwrite.log" 2>&1; then
    printf 'AX process capture unexpectedly replaced its package\n' >&2
    exit 1
fi
grep -Fq 'output directory already exists' "$root/overwrite.log"

assert_rejection() {
    rejected=$(sed -n 's/^retained rejected AX diagnostics at //p' "$1")
    parent=$(CDPATH= cd -- "$root" && pwd -P)
    case $rejected in
        "$parent"/.alpine-ax-rejected.*) ;;
        *) printf 'rejected capture path is absent or foreign\n' >&2; exit 1 ;;
    esac
    test -d "$rejected"
    test ! -e "$rejected/manifest.toml"
    grep -Fxq 'status=rejected' "$rejected/rejection.txt"
    grep -Fxq 'qualified=false' "$rejected/rejection.txt"
    grep -Fxq 'complete=false' "$rejected/rejection.txt"
    grep -Fxq 'fixture_only=true' "$rejected/rejection.txt"
    grep -Fxq "repository_revision=$(git rev-parse HEAD)" "$rejected/rejection.txt"
    grep -Fxq "studio_binary_sha256=$(/usr/bin/shasum -a 256 "$binary" | awk '{print $1}')" \
        "$rejected/rejection.txt"
    case $(uname -s) in
        Darwin) test "$(stat -f %Lp "$rejected")" = 700
            test "$(stat -f %Lp "$rejected/rejection.txt")" = 600 ;;
        *) test "$(stat -c %a "$rejected")" = 700
            test "$(stat -c %a "$rejected/rejection.txt")" = 600 ;;
    esac
    total=0
    tab=$(printf '\t')
    while IFS="$tab" read -r path state observed retained digest truncated; do
        [ "$state" = prefix ] || continue
        actual=$(wc -c < "$rejected/$path" | tr -d '[:space:]')
        test "$actual" -eq "$retained"
        test "$actual" -le 65536
        test "$(/usr/bin/shasum -a 256 "$rejected/$path" | awk '{print $1}')" = "$digest"
        total=$((total + actual))
    done < "$rejected/artifacts.tsv"
    grep -Fxq "retained_payload_bytes=$total" "$rejected/rejection.txt"
    metadata=$(wc -c < "$rejected/rejection.txt" | tr -d '[:space:]')
    index=$(wc -c < "$rejected/artifacts.tsv" | tr -d '[:space:]')
    test "$((metadata + index))" -le 16384
    test "$((total + metadata + index))" -le 1048576
}

expect_failure() {
    label=$1
    binary=$2
    assurance=$3
    sampler=$4
    expected=$5
    if capture "$binary" "$assurance" "$sampler" "$root/$label-package" \
        > "$root/$label.log" 2>&1; then
        printf 'AX process capture control %s unexpectedly passed\n' "$label" >&2
        exit 1
    fi
    grep -Fq "$expected" "$root/$label.log"
    test ! -e "$root/$label-package"
    assert_rejection "$root/$label.log"
}

expect_failure missing-ax "$root/fake-studio" "$root/fake-no-output" \
    "$root/fake-footprint" 'raw AX capture did not publish tree.jsonl'
expect_failure untrusted "$root/fake-studio" "$root/fake-untrusted" \
    "$root/fake-footprint" 'assurance error: AX client is not trusted'
grep -Fq 'raw AX capture command failed' "$root/untrusted.log"
expect_failure sampler-failure "$root/fake-studio" "$root/fake-assurance" \
    "$root/fake-sampler-failure" 'footprint sampler failed'
expect_failure process-failure "$root/fake-studio-failure" "$root/fake-assurance" \
    "$root/fake-footprint" 'Studio process failed with status 7'

expect_failure partial "$root/fake-studio" "$root/fake-partial-failure" \
    "$root/fake-footprint" 'raw AX capture command failed'
printf '%s\n' '{"fixture_only":true,"event":"before-failure"}' > "$root/expected-event"
cmp "$root/expected-event" "$rejected/raw-ax/events.jsonl"
grep -Fxq 'ax_exit_status=17' "$rejected/rejection.txt"
partial_rejected=$rejected
partial_sha=$(/usr/bin/shasum -a 256 "$partial_rejected/raw-ax/events.jsonl" | awk '{print $1}')

expect_failure large "$root/fake-studio" "$root/fake-large-failure" \
    "$root/fake-footprint" 'raw AX capture command failed'
test "$partial_rejected" != "$rejected"
test "$(/usr/bin/shasum -a 256 "$partial_rejected/raw-ax/events.jsonl" | awk '{print $1}')" = "$partial_sha"
test "$(wc -c < "$rejected/raw-ax/events.jsonl" | tr -d '[:space:]')" -eq 65536
awk -F '\t' '$1 == "raw-ax/events.jsonl" { found = ($3 == 131072 && $4 == 65536 && $6 == "true") } END { exit !found }' \
    "$rejected/artifacts.tsv"

mkdir "$root/external"
printf 'private external sentinel\n' > "$root/external/events.jsonl"
external_sha=$(/usr/bin/shasum -a 256 "$root/external/events.jsonl" | awk '{print $1}')
AX_TEST_EXTERNAL_FILE="$(CDPATH= cd -- "$root/external" && pwd -P)/events.jsonl" \
    expect_failure symlink "$root/fake-studio" "$root/fake-symlink-failure" \
    "$root/fake-footprint" 'raw AX capture command failed'
test ! -e "$rejected/raw-ax/events.jsonl"
awk -F '\t' '$1 == "raw-ax/events.jsonl" { found = ($2 == "unsafe_path") } END { exit !found }' \
    "$rejected/artifacts.tsv"
AX_TEST_EXTERNAL_DIRECTORY="$(CDPATH= cd -- "$root/external" && pwd -P)" \
    expect_failure parent-symlink "$root/fake-studio" "$root/fake-parent-symlink-failure" \
    "$root/fake-footprint" 'raw AX capture command failed'
test ! -e "$rejected/raw-ax/events.jsonl"
test "$(/usr/bin/shasum -a 256 "$root/external/events.jsonl" | awk '{print $1}')" = "$external_sha"

(
    AX_TEST_CLOSE_TIMEOUT=1 expect_failure timeout "$root/fake-studio-timeout" \
        "$root/fake-assurance" "$root/fake-footprint" 'Studio remained alive after the post-close timeout'
    grep -Fxq 'phase=close' "$rejected/rejection.txt"
)

AX_TEST_DESTINATION="$(CDPATH= cd -- "$root" && pwd -P)/late-collision-package"
AX_TEST_GOOD_CLIENT="$(CDPATH= cd -- "$root" && pwd -P)/fake-assurance"
export AX_TEST_DESTINATION AX_TEST_GOOD_CLIENT
if capture "$root/fake-studio" "$root/fake-late-collision" "$root/fake-footprint" \
    "$AX_TEST_DESTINATION" > "$root/late-collision.log" 2>&1; then
    printf 'AX capture overwrote a late destination\n' >&2; exit 1
fi
grep -Fq 'output directory appeared during capture' "$root/late-collision.log"
grep -Fxq 'preserve existing output' "$AX_TEST_DESTINATION/sentinel"
test ! -e "$AX_TEST_DESTINATION/manifest.toml"
assert_rejection "$root/late-collision.log"
grep -Fxq 'phase=publication' "$rejected/rejection.txt"
unset AX_TEST_DESTINATION AX_TEST_GOOD_CLIENT

ln -s "$root/absent-target" "$root/dangling-output"
if capture "$root/fake-studio" "$root/fake-assurance" "$root/fake-footprint" \
    "$root/dangling-output" > "$root/dangling.log" 2>&1; then
    printf 'AX capture replaced a dangling output symlink\n' >&2; exit 1
fi
test -L "$root/dangling-output"
grep -Fq 'output directory already exists' "$root/dangling.log"

mkdir "$root/failing-tools"
cat > "$root/failing-tools/head" <<'EOF'
#!/bin/sh
if [ "${1-}" = -c ] && [ "${2-}" = 65536 ]; then
    printf 'injected diagnostic copy failure\n' >&2
    exit 75
fi
exec /usr/bin/head "$@"
EOF
chmod +x "$root/failing-tools/head"
(
if PATH="$(CDPATH= cd -- "$root/failing-tools" && pwd -P):$PATH" \
    capture "$root/fake-studio" "$root/fake-partial-failure" "$root/fake-footprint" \
    "$root/copy-failure-package" > "$root/copy-failure.log" 2>&1; then
    printf 'AX diagnostic copy failure unexpectedly passed\n' >&2; exit 1
fi
grep -Fq 'injected diagnostic copy failure' "$root/copy-failure.log"
original=$(sed -n 's/^failed to retain bounded AX diagnostics; original rejected capture remains at //p' \
    "$root/copy-failure.log")
case $original in
    "$(CDPATH= cd -- "$root" && pwd -P)"/.alpine-ax-process.*) ;;
    *) printf 'failed retention did not preserve its owned source\n' >&2; exit 1 ;;
esac
cmp "$root/expected-event" "$original/raw-ax/events.jsonl"
test ! -e "$root/copy-failure-package"
)

# Inject failure into the hash producer only after the raw capture was retained.
# The copied wrapper keeps the production helper and cleanup path unchanged.
cat > "$root/failing-tools/shasum" <<'EOF'
#!/bin/sh
case $* in
    *'/.alpine-ax-rejected.'*) printf 'injected retained hash failure\n' >&2; exit 74 ;;
esac
exec /usr/bin/shasum "$@"
EOF
chmod +x "$root/failing-tools/shasum"
hash_wrapper="$root/hash-failure-capture.sh"
hash_tool="$(CDPATH= cd -- "$root/failing-tools" && pwd -P)/shasum"
sed "s|/usr/bin/shasum|$hash_tool|g" "$capture_script" > "$hash_wrapper"
chmod +x "$hash_wrapper"
if AX_TEST_CAPTURE_SCRIPT="$hash_wrapper" sh -c '
    exec "$AX_TEST_CAPTURE_SCRIPT" "$@"
' sh --binary "$root/fake-studio" --assurance "$root/fake-partial-failure" \
    --repository . --workspace . --output-dir "$root/hash-failure-package" \
    --generation 1 --pre-action-ms 1 --post-action-ms 1 \
    --duration-seconds 3 --interval-seconds 1 --post-close-timeout-seconds 5 \
    --opt-in --fixture-only --sampler "$root/fake-footprint" \
    > "$root/hash-failure.log" 2>&1; then
    printf 'AX diagnostic hash failure unexpectedly passed\n' >&2; exit 1
fi
grep -Fq 'injected retained hash failure' "$root/hash-failure.log"
original=$(sed -n 's/^failed to retain bounded AX diagnostics; original rejected capture remains at //p' \
    "$root/hash-failure.log")
test -n "$original"
cmp "$root/expected-event" "$original/raw-ax/events.jsonl"
test ! -e "$root/hash-failure-package"

if "$capture_script" --help | \
    grep -Fq 'intermediate Task #504 package'; then
    :
else
    printf 'AX process capture usage is unavailable\n' >&2
    exit 1
fi

printf 'Studio AX process capture checks passed\n'
