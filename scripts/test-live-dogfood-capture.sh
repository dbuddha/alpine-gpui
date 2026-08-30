#!/bin/sh
set -eu

root=target/live-dogfood-capture-contract
rm -rf "$root"
mkdir -p "$root"
revision=$(git rev-parse HEAD)
sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa

cat > "$root/draft.toml" <<EOF
schema = "alpine-studio-dogfood-draft/v2"

[identity]
id = "fixture-live-session"
workload_id = "fixture-live"
workload_version = 1
workspace_fixture = "fixture-workspace"
workspace_fixture_sha256 = "$sha"
settings_profile = "fixture-settings"
settings_sha256 = "$sha"
opt_in = true
telemetry = false
network_io = false
performance_claim = "none"
coverage = ["launch", "workspace", "editing", "language", "accessibility", "lifecycle", "memory", "shutdown"]
assumptions = ["headless fixture process and fake sampler"]
exclusions = ["telemetry", "network-io", "comparative-claim"]

[identity.environment]
hardware_id = "fixture-apple-silicon"
os_build = "fixture"
architecture = "arm64"
display_refresh_hz = 60
power_source = "ac"
thermal_state = "nominal"
toolchain = "fixture"
locale = "en_US.UTF-8"

[identity.font]
family = "SF Mono"
postscript_name = "SFMono-Regular"
size_milli_points = 13000

[identity.language_server]
name = "none"
version = "none"
executable_sha256 = "none"
EOF

cat > "$root/fake-studio" <<'EOF'
#!/bin/sh
sleep 1
cat > "$ALPINE_STUDIO_DOGFOOD_OUTPUT" <<JSON
{
  "schema":"alpine-studio-internal-diagnostic/v1",
  "workload_id":"$ALPINE_STUDIO_DOGFOOD_WORKLOAD_ID",
  "alpine_revision":"$ALPINE_STUDIO_DOGFOOD_REVISION",
  "captured_at_utc":"$ALPINE_STUDIO_DOGFOOD_CAPTURED_AT_UTC",
  "duration_ms":4000,
  "outcome":"passed",
  "status":"clean native close captured",
  "frames":{"requested":2,"submitted":2,"completed":2,"presented":1,"omitted":1,"idle_submissions":0,"peak_in_flight":1},
  "text":{"shape_calls":2,"rasterize_calls":1,"syntax_cache_hits":1,"syntax_cache_misses":1,"syntax_omitted_lines":0},
  "language":{"requests":1,"responses":null,"stale_responses":0,"restarts":0,"current_retained_bytes":8,"peak_retained_bytes":16,"budget_bytes":32},
  "accessibility":{"queries":1,"actions":1,"stale_actions":null,"retained_nodes":2,"peak_retained_nodes":2},
  "lifecycle":{"close_requests":1,"close_completions":1,"clean_shutdown":true,"post_close_bytes":0,"post_close_limit_bytes":0},
  "resources":[
    {"name":"layout-cache","current_bytes":1,"peak_bytes":2,"budget_bytes":4,"omitted":false},
    {"name":"syntax-cache","current_bytes":1,"peak_bytes":2,"budget_bytes":4,"omitted":false},
    {"name":"glyph-atlas-cpu","current_bytes":1,"peak_bytes":2,"budget_bytes":4,"omitted":false},
    {"name":"glyph-atlas-gpu","current_bytes":null,"peak_bytes":null,"budget_bytes":null,"omitted":true},
    {"name":"font-cache","current_bytes":null,"peak_bytes":null,"budget_bytes":null,"omitted":true},
    {"name":"fallback-cache","current_bytes":null,"peak_bytes":null,"budget_bytes":null,"omitted":true},
    {"name":"language-process","current_bytes":8,"peak_bytes":16,"budget_bytes":32,"omitted":false},
    {"name":"foreground-queue","current_bytes":1,"peak_bytes":2,"budget_bytes":4,"omitted":false},
    {"name":"background-queue","current_bytes":null,"peak_bytes":null,"budget_bytes":null,"omitted":true},
    {"name":"upload-staging","current_bytes":1,"peak_bytes":2,"budget_bytes":null,"omitted":false,"omitted_axes":["budget_bytes"]}
  ],
  "runtime":{"stale_results":0},
  "surface":{"current_retained_bytes":0},
  "omissions":["accessibility-stale-actions","background-queue","fallback-cache","font-cache","glyph-atlas-gpu","language-responses","process-gpu-bytes","stage-timings","upload-staging-budget"]
}
JSON
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
        *) exit 2 ;;
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
chmod +x "$root/fake-studio" "$root/fake-footprint"

scripts/capture-studio-dogfood.sh \
    --binary "$root/fake-studio" \
    --repository . \
    --workspace . \
    --draft "$root/draft.toml" \
    --output-dir "$root/bundle" \
    --workload-id fixture-live \
    --duration-seconds 3 \
    --interval-seconds 1 \
    --post-close-timeout-seconds 5 \
    --opt-in --fixture-only --sampler "$root/fake-footprint" \
    > "$root/capture.log"

cargo run --quiet --locked -p alpine-assurance -- \
    validate-studio-dogfood "$root/bundle/session.toml" \
    > "$root/validation.log"
cargo run --quiet --locked -p alpine-assurance -- \
    studio-dogfood-report "$root/bundle/session.toml" \
    > "$root/report.md"
grep -Fq '4 physical samples' "$root/validation.log"
grep -Fq 'Evidence scope: `fixture`' "$root/report.md"
grep -Fq 'GPU process sampling: omitted, unavailable' "$root/report.md"
grep -Fq 'gpu_process_sampling = "omitted-unavailable"' "$root/bundle/session.toml"
if grep -Fq 'gpu_bytes' "$root/bundle/snapshot.toml"; then
    printf 'live snapshot fabricated unavailable GPU bytes\n' >&2
    exit 1
fi
for artifact in internal-diagnostic.json footprint.json studio.stdout studio.stderr snapshot.toml; do
    test -f "$root/bundle/$artifact"
done

if scripts/capture-studio-dogfood.sh \
    --binary "$root/fake-studio" --repository . --workspace . \
    --draft "$root/draft.toml" --output-dir "$root/bundle" \
    --workload-id fixture-live --duration-seconds 3 --interval-seconds 1 \
    --post-close-timeout-seconds 5 --opt-in --fixture-only \
    --sampler "$root/fake-footprint" > "$root/overwrite.log" 2>&1; then
    printf 'live capture unexpectedly overwrote an existing bundle\n' >&2
    exit 1
fi
grep -Fq 'output directory already exists' "$root/overwrite.log"

cp -R "$root/bundle" "$root/tampered"
printf 'tampered\n' >> "$root/tampered/footprint.json"
if cargo run --quiet --locked -p alpine-assurance -- \
    validate-studio-dogfood "$root/tampered/session.toml" \
    > "$root/tamper.log" 2>&1; then
    printf 'tampered live capture unexpectedly validated\n' >&2
    exit 1
fi
grep -Fq 'SHA-256 differs' "$root/tamper.log"

cat > "$root/fake-no-output" <<'EOF'
#!/bin/sh
sleep 1
exit 0
EOF
cat > "$root/fake-failure" <<'EOF'
#!/bin/sh
sleep 1
exit 7
EOF
cat > "$root/fake-drift" <<'EOF'
#!/bin/sh
ALPINE_STUDIO_DOGFOOD_REVISION=cccccccccccccccccccccccccccccccccccccccc
export ALPINE_STUDIO_DOGFOOD_REVISION
exec "$(dirname "$0")/fake-studio" "$@"
EOF
cat > "$root/fake-malformed-footprint" <<'EOF'
#!/bin/sh
output=
while [ "$#" -gt 0 ]; do
    case $1 in
        --json) output=$2; shift 2 ;;
        --pid|--sample|--sample-duration|--format) shift 2 ;;
        --noCategories) shift ;;
        *) exit 2 ;;
    esac
done
printf '{malformed\n' > "$output"
EOF
chmod +x "$root/fake-no-output" "$root/fake-failure" \
    "$root/fake-drift" "$root/fake-malformed-footprint"

expect_failure() {
    label=$1
    binary=$2
    sampler=$3
    expected=$4
    destination="$root/$label-bundle"
    if scripts/capture-studio-dogfood.sh \
        --binary "$binary" --repository . --workspace . \
        --draft "$root/draft.toml" --output-dir "$destination" \
        --workload-id fixture-live --duration-seconds 3 --interval-seconds 1 \
        --post-close-timeout-seconds 5 --opt-in --fixture-only \
        --sampler "$sampler" > "$root/$label.log" 2>&1; then
        printf 'live capture control %s unexpectedly passed\n' "$label" >&2
        exit 1
    fi
    grep -Fq "$expected" "$root/$label.log"
    test ! -e "$destination"
}

expect_failure missing-internal "$root/fake-no-output" \
    "$root/fake-footprint" 'did not publish its internal diagnostic'
expect_failure process-failure "$root/fake-failure" \
    "$root/fake-footprint" 'Studio process failed'
expect_failure malformed-sampler "$root/fake-studio" \
    "$root/fake-malformed-footprint" 'cannot parse footprint JSON'
expect_failure revision-drift "$root/fake-drift" \
    "$root/fake-footprint" 'expected repository revision'

printf 'live Studio dogfood capture checks passed\n'
