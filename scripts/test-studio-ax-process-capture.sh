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

chmod +x "$root/fake-studio" "$root/fake-assurance" \
    "$root/fake-no-output" "$root/fake-untrusted" "$root/fake-footprint" \
    "$root/fake-sampler-failure" "$root/fake-studio-failure"

capture() {
    binary=$1
    assurance=$2
    sampler=$3
    destination=$4
    scripts/capture-studio-ax-process.sh \
        --binary "$binary" --assurance "$assurance" \
        --repository . --workspace . --output-dir "$destination" \
        --generation 1 --pre-action-ms 1 --post-action-ms 1 \
        --duration-seconds 3 --interval-seconds 1 \
        --post-close-timeout-seconds 5 --opt-in --fixture-only \
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

if capture "$root/fake-studio" "$root/fake-assurance" \
    "$root/fake-footprint" "$root/package" > "$root/overwrite.log" 2>&1; then
    printf 'AX process capture unexpectedly replaced its package\n' >&2
    exit 1
fi
grep -Fq 'output directory already exists' "$root/overwrite.log"

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

if scripts/capture-studio-ax-process.sh --help | \
    grep -Fq 'intermediate Task #504 package'; then
    :
else
    printf 'AX process capture usage is unavailable\n' >&2
    exit 1
fi

printf 'Studio AX process capture checks passed\n'
