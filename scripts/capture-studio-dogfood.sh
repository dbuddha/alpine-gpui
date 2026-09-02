#!/bin/sh
set -eu

usage() {
    cat <<'EOF'
usage: capture-studio-dogfood.sh \
  --binary PATH --repository PATH --workspace PATH --draft PATH \
  --output-dir PATH --workload-id SLUG --duration-seconds N \
  --interval-seconds N --post-close-timeout-seconds N --opt-in \
  [--fixture-only --sampler PATH]

Normal capture requires Apple Silicon macOS and uses /usr/bin/footprint.
Fixture mode is headless, is marked non-physical, and permits a fake sampler.
EOF
}

fail() {
    printf 'live Studio dogfood capture failed: %s\n' "$1" >&2
    exit 1
}

is_uint() {
    case $1 in
        '' | *[!0-9]*) return 1 ;;
        *) return 0 ;;
    esac
}

canonical_file() {
    directory=$(CDPATH= cd -- "$(dirname -- "$1")" && pwd -P) || return 1
    printf '%s/%s\n' "$directory" "$(basename -- "$1")"
}

binary=
repository=
workspace=
draft=
output_dir=
workload_id=
duration=
interval=
post_close_timeout=
opt_in=false
fixture=false
sampler=

while [ "$#" -gt 0 ]; do
    case $1 in
        --binary) binary=${2-}; shift 2 ;;
        --repository) repository=${2-}; shift 2 ;;
        --workspace) workspace=${2-}; shift 2 ;;
        --draft) draft=${2-}; shift 2 ;;
        --output-dir) output_dir=${2-}; shift 2 ;;
        --workload-id) workload_id=${2-}; shift 2 ;;
        --duration-seconds) duration=${2-}; shift 2 ;;
        --interval-seconds) interval=${2-}; shift 2 ;;
        --post-close-timeout-seconds) post_close_timeout=${2-}; shift 2 ;;
        --opt-in) opt_in=true; shift ;;
        --fixture-only) fixture=true; shift ;;
        --sampler) sampler=${2-}; shift 2 ;;
        --help|-h) usage; exit 0 ;;
        *) usage >&2; fail "unknown or incomplete argument: $1" ;;
    esac
done

[ "$opt_in" = true ] || fail "capture requires the explicit --opt-in flag"
[ -x "$binary" ] || fail "binary must identify an executable file"
[ -d "$repository" ] || fail "repository must identify a directory"
[ -e "$workspace" ] || fail "workspace fixture path is unavailable"
[ -f "$draft" ] || fail "draft must identify a TOML file"
[ -n "$output_dir" ] || fail "output directory is required"
[ ! -e "$output_dir" ] || fail "output directory already exists"
case $workload_id in
    '' | *[!a-z0-9.-]*) fail "workload id must be a lowercase bounded slug" ;;
esac
[ "${#workload_id}" -le 128 ] || fail "workload id exceeds 128 bytes"
for value in "$duration" "$interval" "$post_close_timeout"; do
    is_uint "$value" || fail "durations and intervals must be whole seconds"
done
[ "$duration" -gt "$interval" ] || fail "duration must exceed interval"
[ "$interval" -gt 0 ] || fail "interval must be positive"
[ "$post_close_timeout" -gt 0 ] || fail "post-close timeout must be positive"
sample_bound=$((duration / interval + 2))
[ "$sample_bound" -ge 4 ] && [ "$sample_bound" -le 4096 ] ||
    fail "capture sample bound must remain within 4 and 4096"

evidence_scope=physical
if [ "$fixture" = true ]; then
    evidence_scope=fixture
    [ -x "$sampler" ] || fail "fixture mode requires an executable --sampler"
else
    [ "$(uname -s)" = Darwin ] && [ "$(uname -m)" = arm64 ] ||
        fail "physical capture requires Apple Silicon macOS"
    [ -z "$sampler" ] || fail "physical capture does not accept a replacement sampler"
    sampler=/usr/bin/footprint
    [ -x "$sampler" ] || fail "Apple footprint is unavailable"
fi

repository=$(CDPATH= cd -- "$repository" && pwd -P) || fail "repository path is unavailable"
revision=$(git -C "$repository" rev-parse HEAD 2>/dev/null) || fail "repository revision is unavailable"
printf '%s' "$revision" | grep -Eq '^[0-9a-f]{40}$' || fail "repository revision is not a full Git SHA"
if [ "$fixture" != true ]; then
    [ -z "$(git -C "$repository" status --porcelain --untracked-files=normal)" ] ||
        fail "repository must be clean for revision-bound capture"
fi
binary=$(canonical_file "$binary") || fail "binary canonical path is unavailable"
sampler=$(canonical_file "$sampler") || fail "sampler canonical path is unavailable"
draft=$(canonical_file "$draft") || fail "draft canonical path is unavailable"
if [ "$fixture" != true ]; then
    contents=$(CDPATH= cd -- "$(dirname -- "$binary")/.." && pwd -P) ||
        fail "release bundle Contents directory is unavailable"
    build_identity="$contents/Resources/alpine-build-identity.toml"
    [ -f "$build_identity" ] || fail "release bundle build identity is unavailable"
    grep -Fq "revision = \"$revision\"" "$build_identity" ||
        fail "release bundle revision does not match repository HEAD"
    binary_sha=$(shasum -a 256 "$binary" | awk '{print $1}')
    grep -Fq "executable_sha256 = \"$binary_sha\"" "$build_identity" ||
        fail "release bundle executable hash does not match its build identity"
fi
workspace_parent=$(CDPATH= cd -- "$(dirname -- "$workspace")" && pwd -P) ||
    fail "workspace canonical parent is unavailable"
workspace="$workspace_parent/$(basename -- "$workspace")"

output_parent=$(dirname -- "$output_dir")
[ -d "$output_parent" ] || fail "output parent must exist"
output_parent=$(CDPATH= cd -- "$output_parent" && pwd -P) || fail "output parent is unavailable"
output_dir="$output_parent/$(basename -- "$output_dir")"
capture_root=$(mktemp -d "$output_parent/.alpine-live-capture.XXXXXX")
studio_pid=
sampler_pid=
cleanup() {
    if [ -n "${sampler_pid-}" ] && kill -0 "$sampler_pid" 2>/dev/null; then
        kill "$sampler_pid" 2>/dev/null || true
    fi
    if [ "$fixture" = true ] && [ -n "${studio_pid-}" ] && kill -0 "$studio_pid" 2>/dev/null; then
        kill "$studio_pid" 2>/dev/null || true
    fi
    rm -rf "$capture_root"
}
trap cleanup EXIT HUP INT TERM

internal="$capture_root/internal-diagnostic.json"
stdout="$capture_root/studio.stdout"
stderr="$capture_root/studio.stderr"
raw_footprint="$capture_root/footprint.json"
captured_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)

ALPINE_STUDIO_DOGFOOD_OUTPUT="$internal" \
ALPINE_STUDIO_DOGFOOD_WORKLOAD_ID="$workload_id" \
ALPINE_STUDIO_DOGFOOD_REVISION="$revision" \
ALPINE_STUDIO_DOGFOOD_CAPTURED_AT_UTC="$captured_at" \
    "$binary" "$workspace" > "$stdout" 2> "$stderr" &
studio_pid=$!
captured_pid=$studio_pid
kill -0 "$studio_pid" 2>/dev/null || fail "Studio process did not remain alive after launch"

if [ "$fixture" = true ]; then
    process_start=fixture-headless-process
else
    running_binary=$(ps -ww -p "$studio_pid" -o comm= | sed 's/^[[:space:]]*//')
    [ -n "$running_binary" ] || fail "Studio process executable identity is unavailable"
    running_binary=$(canonical_file "$running_binary") || fail "running executable path is unavailable"
    [ "$running_binary" = "$binary" ] || fail "running process does not match the declared binary"
    process_start=$(ps -ww -p "$studio_pid" -o lstart= | sed 's/^[[:space:]]*//')
    [ -n "$process_start" ] || fail "Studio process start identity is unavailable"
fi

"$sampler" --pid "$studio_pid" --sample "$interval" \
    --sample-duration "$duration" --noCategories --format bytes \
    --json "$raw_footprint" > "$capture_root/footprint.log" 2>&1 &
sampler_pid=$!
set +e
wait "$sampler_pid"
sampler_status=$?
set -e
sampler_pid=
[ "$sampler_status" -eq 0 ] || fail "footprint sampler failed"
[ -f "$raw_footprint" ] || fail "footprint sampler did not publish machine-readable JSON"

if [ "$fixture" != true ]; then
    running_binary=$(ps -ww -p "$studio_pid" -o comm= | sed 's/^[[:space:]]*//')
    [ -n "$running_binary" ] || fail "Studio exited before the sampling window completed"
    running_binary=$(canonical_file "$running_binary") || fail "post-sample executable path is unavailable"
    [ "$running_binary" = "$binary" ] || fail "Studio process identity drifted during capture"
    current_start=$(ps -ww -p "$studio_pid" -o lstart= | sed 's/^[[:space:]]*//')
    [ "$current_start" = "$process_start" ] || fail "Studio process start identity drifted"
    printf 'sampling complete; request application Quit or close the Alpine Studio window within %s seconds; a blocked close remains live\n' "$post_close_timeout"
fi

remaining=$post_close_timeout
while kill -0 "$studio_pid" 2>/dev/null && [ "$remaining" -gt 0 ]; do
    sleep 1
    remaining=$((remaining - 1))
done
if kill -0 "$studio_pid" 2>/dev/null; then
    fail "Studio remained alive after the post-close timeout"
fi
set +e
wait "$studio_pid"
studio_status=$?
set -e
studio_pid=
[ "$studio_status" -eq 0 ] || fail "Studio process failed"
[ -f "$internal" ] || fail "Studio did not publish its internal diagnostic"

cargo run --quiet --locked --manifest-path "$repository/Cargo.toml" \
    -p alpine-assurance -- seal-live-studio-dogfood \
    "$draft" "$internal" "$raw_footprint" "$stdout" "$stderr" \
    "$binary" "$sampler" "$captured_pid" \
    "$((duration * 1000))" "$((interval * 1000))" "$evidence_scope" \
    "$process_start" "$revision" "$captured_at" "$output_dir"

printf 'retained live Studio dogfood bundle at %s\n' "$output_dir"
