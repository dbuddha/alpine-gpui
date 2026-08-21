#!/bin/sh
set -eu

usage() {
    cat <<'EOF'
usage: capture-studio-residency.sh \
  --pid PID --binary PATH --repository PATH --output-dir PATH \
  --revision GIT_SHA --workload PATH --environment PATH \
  --duration-seconds N --interval-seconds N --warmup-seconds N \
  --post-close-timeout-seconds N [--slope-limit-bytes-per-second N]
EOF
}

fail() {
    printf 'studio residency capture failed: %s\n' "$1" >&2
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

pid=
binary=
repository=
output_dir=
revision=
workload=
environment=
duration=
interval=
warmup=
post_close_timeout=
slope_limit=

while [ "$#" -gt 0 ]; do
    case $1 in
        --pid) pid=${2-}; shift 2 ;;
        --binary) binary=${2-}; shift 2 ;;
        --repository) repository=${2-}; shift 2 ;;
        --output-dir) output_dir=${2-}; shift 2 ;;
        --revision) revision=${2-}; shift 2 ;;
        --workload) workload=${2-}; shift 2 ;;
        --environment) environment=${2-}; shift 2 ;;
        --duration-seconds) duration=${2-}; shift 2 ;;
        --interval-seconds) interval=${2-}; shift 2 ;;
        --warmup-seconds) warmup=${2-}; shift 2 ;;
        --post-close-timeout-seconds) post_close_timeout=${2-}; shift 2 ;;
        --slope-limit-bytes-per-second) slope_limit=${2-}; shift 2 ;;
        --help|-h) usage; exit 0 ;;
        *) usage >&2; fail "unknown or incomplete argument: $1" ;;
    esac
done

[ "$(uname -s)" = Darwin ] && [ "$(uname -m)" = arm64 ] ||
    fail "physical capture requires Apple Silicon macOS"
is_uint "$pid" || fail "PID must be an unsigned integer"
[ -x "$binary" ] || fail "binary must identify an executable file"
[ -d "$repository" ] || fail "repository must identify a directory"
[ -f "$workload" ] || fail "workload must identify a retained record"
[ -f "$environment" ] || fail "environment must identify a retained record"
[ -n "$output_dir" ] || fail "output directory is required"
[ ! -e "$output_dir" ] || fail "output directory already exists"
printf '%s' "$revision" | grep -Eq '^[0-9a-f]{40}$' ||
    fail "revision must be a full lowercase Git SHA"
repository_revision=$(git -C "$repository" rev-parse HEAD 2>/dev/null) ||
    fail "repository revision is unavailable"
[ "$repository_revision" = "$revision" ] ||
    fail "repository HEAD does not match the declared revision"
[ -z "$(git -C "$repository" status --porcelain)" ] ||
    fail "repository must be clean for revision-bound capture"
for value in "$duration" "$interval" "$warmup" "$post_close_timeout"; do
    is_uint "$value" || fail "durations and intervals must be whole seconds"
done
[ "$duration" -gt "$interval" ] || fail "duration must exceed interval"
[ "$interval" -gt 0 ] || fail "interval must be positive"
[ "$warmup" -lt "$duration" ] || fail "warmup must be shorter than duration"
[ "$post_close_timeout" -gt 0 ] || fail "post-close timeout must be positive"
if [ -n "$slope_limit" ]; then
    is_uint "$slope_limit" || fail "slope limit must be unsigned bytes per second"
fi
sample_bound=$((duration / interval + 2))
[ "$sample_bound" -le 4096 ] || fail "capture could exceed 4096 samples"
[ "$sample_bound" -ge 4 ] || fail "capture must admit at least four samples"
kill -0 "$pid" 2>/dev/null || fail "target process is not running"

declared_binary=$(canonical_file "$binary") ||
    fail "binary canonical path is unavailable"
running_binary=$(ps -ww -p "$pid" -o comm= | sed 's/^[[:space:]]*//')
[ -n "$running_binary" ] || fail "target process executable is unavailable"
[ "${running_binary#/}" != "$running_binary" ] ||
    fail "target process did not expose an absolute executable path"
running_binary=$(canonical_file "$running_binary") ||
    fail "target process canonical path is unavailable"
[ "$running_binary" = "$declared_binary" ] ||
    fail "PID executable does not match the declared binary"
process_start=$(ps -ww -p "$pid" -o lstart= | sed 's/^[[:space:]]*//')
[ -n "$process_start" ] || fail "target process start identity is unavailable"

mkdir -p "$output_dir"
cp "$workload" "$output_dir/workload-record"
cp "$environment" "$output_dir/environment-record"
workload_hash=$(/usr/bin/shasum -a 256 "$output_dir/workload-record" | awk '{print $1}')
environment_hash=$(/usr/bin/shasum -a 256 "$output_dir/environment-record" | awk '{print $1}')
binary_sha=$(/usr/bin/shasum -a 256 "$declared_binary" | awk '{print $1}')
start_epoch=$(date +%s)
os_build=$(sw_vers -buildVersion)
hardware_model=$(sysctl -n hw.model)
manifest="$output_dir/manifest.toml"

cat > "$manifest" <<EOF
schema = "alpine-studio-residency-capture/v1"
revision = "$revision"
workload_sha256 = "$workload_hash"
environment_sha256 = "$environment_hash"
binary_sha256 = "$binary_sha"
binary_name = "$(basename "$declared_binary")"
pid = $pid
process_start = "$process_start"
duration_seconds = $duration
interval_seconds = $interval
warmup_seconds = $warmup
sample_bound = $sample_bound
start_epoch_seconds = $start_epoch
os_build = "$os_build"
hardware_model = "$hardware_model"
child_process_scope = "excluded; capture language servers separately"
EOF

set +e
/usr/bin/footprint --pid "$pid" --sample "$interval" \
    --sample-duration "$duration" --noCategories --format bytes \
    --json "$output_dir/footprint.json" > "$output_dir/footprint.log" 2>&1
footprint_exit=$?
set -e

analysis_exit=2
if [ "$footprint_exit" -eq 0 ]; then
    set +e
    "$(dirname "$0")/analyze-studio-residency.sh" \
        "$output_dir/footprint.json" "$pid" "$warmup" \
        "$output_dir/analysis" ${slope_limit:+"$slope_limit"}
    analysis_exit=$?
    set -e
fi

printf 'sampling complete; close Alpine Studio within %s seconds\n' \
    "$post_close_timeout"
remaining=$post_close_timeout
while kill -0 "$pid" 2>/dev/null && [ "$remaining" -gt 0 ]; do
    sleep 1
    remaining=$((remaining - 1))
done
post_close_observed=true
if kill -0 "$pid" 2>/dev/null; then
    post_close_observed=false
fi

end_epoch=$(date +%s)
raw_sha=unavailable
[ ! -f "$output_dir/footprint.json" ] ||
    raw_sha=$(/usr/bin/shasum -a 256 "$output_dir/footprint.json" | awk '{print $1}')
analysis_sha=unavailable
[ ! -f "$output_dir/analysis/summary.toml" ] ||
    analysis_sha=$(/usr/bin/shasum -a 256 "$output_dir/analysis/summary.toml" |
        awk '{print $1}')
capture_status=complete
if [ "$footprint_exit" -ne 0 ] || [ "$analysis_exit" -ne 0 ] ||
    [ "$post_close_observed" != true ]; then
    capture_status=failed
fi
cat >> "$manifest" <<EOF
end_epoch_seconds = $end_epoch
raw_footprint_sha256 = "$raw_sha"
analysis_sha256 = "$analysis_sha"
footprint_exit_code = $footprint_exit
analysis_exit_code = $analysis_exit
post_close_observed = $post_close_observed
capture_status = "$capture_status"
EOF

[ "$footprint_exit" -eq 0 ] || fail "Apple footprint capture failed"
[ "$post_close_observed" = true ] ||
    fail "target process remained alive after the post-close timeout"
[ "$analysis_exit" -eq 0 ] ||
    fail "residency window exceeded its calibrated slope gate"
printf 'retained Studio residency capture at %s\n' "$output_dir"
