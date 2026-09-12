#!/bin/sh
set -eu

usage() {
    cat <<'EOF'
usage: capture-studio-ax-process.sh \
  --binary PATH --assurance PATH --repository PATH --workspace PATH \
  --output-dir PATH --generation N --pre-action-ms N --post-action-ms N \
  --duration-seconds N --interval-seconds N \
  --post-close-timeout-seconds N --opt-in \
  [--fixture-only --sampler PATH]

Normal capture requires a clean Apple Silicon macOS checkout and uses
/usr/bin/footprint. Fixture mode is non-physical and requires explicit fake
assurance and sampler executables. The output is an intermediate Task #504 package
and is not AEP-0273 physical qualification evidence.

Rejected captures retain private, bounded diagnostics beside the requested
output, never a successful package. Diagnostic copies may be truncated and
cannot qualify a physical journey. If retention fails, the original temporary
capture is left in place and its path is reported.
EOF
}

fail() {
    failure_reason=$1
    printf 'Studio AX process capture failed: %s\n' "$1" >&2
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

sha256() {
    hash_output=$(/usr/bin/shasum -a 256 "$1") || return 1
    hash_digest=$(printf '%s\n' "$hash_output" | awk '{print $1}') || return 1
    [ "${#hash_digest}" -eq 64 ] || return 1
    case $hash_digest in *[!0-9a-f]*) return 1 ;; esac
    printf '%s\n' "$hash_digest"
}

stream_sha256() {
    /usr/bin/shasum -a 256 | awk '{print $1}'
}

artifact() {
    section=$1
    relative=$2
    path="$capture_root/$relative"
    [ -f "$path" ] || fail "artifact $relative is unavailable"
    bytes=$(wc -c < "$path" | tr -d '[:space:]')
    digest=$(sha256 "$path")
    cat >> "$manifest" <<EOF

[artifacts.$section]
path = "$relative"
sha256 = "$digest"
bytes = $bytes
EOF
}

retain_rejected_capture() {
    [ ! -L "$capture_root" ] || return 1
    umask 077
    rejected_root=$(mktemp -d "$output_parent/.alpine-ax-rejected.XXXXXX") || return 1
    mkdir "$rejected_root/raw-ax" "$rejected_root/residency-analysis" || return 1
    rejected_file_limit=65536
    rejected_metadata_limit=16384
    rejected_total_limit=1048576
    rejected_payload_bytes=0
    cat > "$rejected_root/rejection.txt" <<EOF
format=alpine-ax-rejection-diagnostic-v1
status=rejected
qualified=false
complete=false
performance_claim=false
aep_0273_bundle_ready=false
producer_quiescence=not_qualified
fixture_only=$fixture
wrapper_exit_status=$cleanup_status
phase=$capture_phase
repository_revision=$revision
repository_clean=$repository_clean
repository_status_sha256=$repository_status_sha
workspace_identity_sha256=$workspace_identity_sha
studio_binary_sha256=$capture_binary_sha
harness_binary_sha256=$capture_harness_sha
sampler_sha256=$capture_sampler_sha
studio_pid=${captured_pid-unavailable}
sampler_pid=${captured_sampler_pid-unavailable}
ax_exit_status=${ax_status-unavailable}
sampler_exit_status=${sampler_status-unavailable}
studio_exit_status=${studio_status-unavailable}
artifact_limit_bytes=$rejected_file_limit
metadata_limit_bytes=$rejected_metadata_limit
total_limit_bytes=$rejected_total_limit
EOF
    [ "$?" -eq 0 ] || return 1
    printf 'reason=' >> "$rejected_root/rejection.txt" || return 1
    printf '%s' "$failure_reason" | head -c 4096 >> "$rejected_root/rejection.txt" || return 1
    printf '\nprocess_start=' >> "$rejected_root/rejection.txt" || return 1
    printf '%s' "${process_start-unavailable}" | head -c 256 >> "$rejected_root/rejection.txt" || return 1
    printf '\n' >> "$rejected_root/rejection.txt" || return 1
    printf 'path\tstate\tobserved_bytes\tretained_bytes\tsha256\ttruncated\n' \
        > "$rejected_root/artifacts.tsv" || return 1

    # Only fixed capture-owned paths are eligible. A prefix is diagnostic input,
    # not an atomic snapshot or a hash of the complete original artifact.
    for rejected_relative in raw-ax/tree.jsonl raw-ax/events.jsonl \
        raw-ax/latency.jsonl footprint.json footprint.log \
        residency-analysis/samples.csv residency-analysis/summary.toml \
        residency.log ax.stdout ax.stderr studio.stdout studio.stderr \
        workspace-record.txt; do
        rejected_source="$capture_root/$rejected_relative"
        rejected_parent=$(dirname -- "$rejected_source")
        if [ -L "$rejected_source" ] || [ -L "$rejected_parent" ]; then
            printf '%s\tunsafe_path\tunavailable\tunavailable\tunavailable\tunavailable\n' \
                "$rejected_relative" >> "$rejected_root/artifacts.tsv" || return 1
        elif [ ! -f "$rejected_source" ]; then
            printf '%s\tunavailable\tunavailable\tunavailable\tunavailable\tunavailable\n' \
                "$rejected_relative" >> "$rejected_root/artifacts.tsv" || return 1
        else
            rejected_observed=$(wc -c < "$rejected_source" | tr -d '[:space:]')
            is_uint "$rejected_observed" || return 1
            head -c "$rejected_file_limit" "$rejected_source" \
                > "$rejected_root/$rejected_relative" || return 1
            rejected_bytes=$(wc -c < "$rejected_root/$rejected_relative" | tr -d '[:space:]')
            is_uint "$rejected_bytes" || return 1
            [ "$rejected_bytes" -le "$rejected_file_limit" ] || return 1
            rejected_digest=$(sha256 "$rejected_root/$rejected_relative") || return 1
            rejected_truncated=false
            [ "$rejected_observed" -le "$rejected_bytes" ] || rejected_truncated=true
            printf '%s\tprefix\t%s\t%s\t%s\t%s\n' "$rejected_relative" \
                "$rejected_observed" "$rejected_bytes" "$rejected_digest" \
                "$rejected_truncated" >> "$rejected_root/artifacts.tsv" || return 1
            rejected_payload_bytes=$((rejected_payload_bytes + rejected_bytes))
        fi
    done
    printf 'retained_payload_bytes=%s\n' "$rejected_payload_bytes" \
        >> "$rejected_root/rejection.txt" || return 1
    rejected_metadata_bytes=$(
        wc -c < "$rejected_root/rejection.txt" | tr -d '[:space:]'
    )
    rejected_index_bytes=$(
        wc -c < "$rejected_root/artifacts.tsv" | tr -d '[:space:]'
    )
    is_uint "$rejected_metadata_bytes" && is_uint "$rejected_index_bytes" || return 1
    rejected_metadata_bytes=$((rejected_metadata_bytes + rejected_index_bytes))
    [ "$rejected_metadata_bytes" -le "$rejected_metadata_limit" ] || return 1
    [ "$((rejected_payload_bytes + rejected_metadata_bytes))" -le "$rejected_total_limit" ] || return 1
    printf 'retained rejected AX diagnostics at %s\n' "$rejected_root" >&2
}

binary=
assurance=
repository=
workspace=
output_dir=
generation=
pre_action_ms=
post_action_ms=
duration=
interval=
post_close_timeout=
opt_in=false
fixture=false
sampler=
failure_reason=unhandled_failure

while [ "$#" -gt 0 ]; do
    case $1 in
        --binary) binary=${2-}; shift 2 ;;
        --assurance) assurance=${2-}; shift 2 ;;
        --repository) repository=${2-}; shift 2 ;;
        --workspace) workspace=${2-}; shift 2 ;;
        --output-dir) output_dir=${2-}; shift 2 ;;
        --generation) generation=${2-}; shift 2 ;;
        --pre-action-ms) pre_action_ms=${2-}; shift 2 ;;
        --post-action-ms) post_action_ms=${2-}; shift 2 ;;
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
[ -x "$assurance" ] || fail "assurance must identify an executable file"
[ -d "$repository" ] || fail "repository must identify a directory"
[ -e "$workspace" ] || fail "workspace path is unavailable"
[ -n "$output_dir" ] || fail "output directory is required"
[ ! -e "$output_dir" ] && [ ! -L "$output_dir" ] || fail "output directory already exists"
for value in "$generation" "$pre_action_ms" "$post_action_ms" \
    "$duration" "$interval" "$post_close_timeout"; do
    is_uint "$value" || fail "identity and duration values must be unsigned integers"
done
[ "$generation" -gt 0 ] || fail "generation must be positive"
[ "$pre_action_ms" -gt 0 ] && [ "$pre_action_ms" -le 120000 ] ||
    fail "pre-action duration must be between 1 and 120000 milliseconds"
[ "$post_action_ms" -gt 0 ] && [ "$post_action_ms" -le 120000 ] ||
    fail "post-action duration must be between 1 and 120000 milliseconds"
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

repository=$(CDPATH= cd -- "$repository" && pwd -P) ||
    fail "repository path is unavailable"
revision=$(git -C "$repository" rev-parse HEAD 2>/dev/null) ||
    fail "repository revision is unavailable"
printf '%s' "$revision" | grep -Eq '^[0-9a-f]{40}$' ||
    fail "repository revision is not a full Git SHA"
repository_status=$(git -C "$repository" status --porcelain --untracked-files=normal) ||
    fail "repository status is unavailable"
repository_clean=true
[ -z "$repository_status" ] || repository_clean=false
if [ "$fixture" != true ] && [ "$repository_clean" != true ]; then
    fail "repository must be clean for revision-bound physical capture"
fi
repository_status_sha=$(printf '%s' "$repository_status" | stream_sha256)

binary=$(canonical_file "$binary") || fail "binary canonical path is unavailable"
assurance=$(canonical_file "$assurance") || fail "assurance canonical path is unavailable"
sampler=$(canonical_file "$sampler") || fail "sampler canonical path is unavailable"
if [ -d "$workspace" ]; then
    workspace=$(CDPATH= cd -- "$workspace" && pwd -P) ||
        fail "workspace canonical path is unavailable"
    workspace_kind=directory
else
    workspace=$(canonical_file "$workspace") || fail "workspace canonical path is unavailable"
    workspace_kind=file
fi

if [ "$fixture" != true ]; then
    contents=$(CDPATH= cd -- "$(dirname -- "$binary")/.." && pwd -P) ||
        fail "release bundle Contents directory is unavailable"
    build_identity="$contents/Resources/alpine-build-identity.toml"
    [ -f "$build_identity" ] || fail "release bundle build identity is unavailable"
    grep -Fq "revision = \"$revision\"" "$build_identity" ||
        fail "release bundle revision does not match repository HEAD"
    grep -Fq "executable_sha256 = \"$(sha256 "$binary")\"" "$build_identity" ||
        fail "release bundle executable hash does not match its build identity"
fi

workspace_path_sha=$(printf '%s' "$workspace" | stream_sha256)
workspace_revision=none
workspace_status_sha=$(printf '' | stream_sha256)
if [ "$workspace_kind" = file ]; then
    workspace_identity_sha=$(sha256 "$workspace")
elif git -C "$workspace" rev-parse HEAD >/dev/null 2>&1; then
    workspace_kind=git-directory
    workspace_revision=$(git -C "$workspace" rev-parse HEAD)
    workspace_status=$(git -C "$workspace" status --porcelain --untracked-files=normal)
    workspace_status_sha=$(printf '%s' "$workspace_status" | stream_sha256)
    workspace_identity_sha=$(
        printf '%s\n%s\n%s\n' "$workspace_revision" "$workspace_status_sha" \
            "$workspace_path_sha" | stream_sha256
    )
elif [ "$fixture" = true ]; then
    workspace_kind=fixture-directory
    workspace_identity_sha=$workspace_path_sha
else
    fail "physical directory workspace must belong to a Git repository"
fi

output_parent=$(dirname -- "$output_dir")
[ -d "$output_parent" ] || fail "output parent must exist"
output_parent=$(CDPATH= cd -- "$output_parent" && pwd -P) ||
    fail "output parent is unavailable"
output_dir="$output_parent/$(basename -- "$output_dir")"
[ ! -e "$output_dir" ] && [ ! -L "$output_dir" ] || fail "output directory already exists"
capture_binary_sha=$(sha256 "$binary") || fail "Studio executable hash is unavailable"
capture_harness_sha=$(sha256 "$assurance") || fail "AX client executable hash is unavailable"
capture_sampler_sha=$(sha256 "$sampler") || fail "sampler executable hash is unavailable"
capture_root=$(mktemp -d "$output_parent/.alpine-ax-process.XXXXXX")
studio_pid=
sampler_pid=
published=false
capture_phase=launch
cleanup() {
    cleanup_status=$?
    trap - EXIT HUP INT TERM
    set +e
    if [ -n "${sampler_pid-}" ] && kill -0 "$sampler_pid" 2>/dev/null; then
        kill "$sampler_pid" 2>/dev/null || true
    fi
    if [ -n "${studio_pid-}" ] && kill -0 "$studio_pid" 2>/dev/null; then
        kill "$studio_pid" 2>/dev/null || true
    fi
    if [ "$published" != true ] && [ -d "${capture_root-}" ]; then
        [ "$cleanup_status" -ne 0 ] || cleanup_status=1
        if retain_rejected_capture; then
            rm -rf "$capture_root"
        else
            printf 'failed to retain bounded AX diagnostics; original rejected capture remains at %s\n' \
                "$capture_root" >&2
        fi
    fi
    exit "$cleanup_status"
}
trap cleanup EXIT
trap 'failure_reason="signal HUP"; exit 129' HUP
trap 'failure_reason="signal INT"; exit 130' INT
trap 'failure_reason="signal TERM"; exit 143' TERM

studio_stdout="$capture_root/studio.stdout"
studio_stderr="$capture_root/studio.stderr"
ax_stdout="$capture_root/ax.stdout"
ax_stderr="$capture_root/ax.stderr"
raw_footprint="$capture_root/footprint.json"
capture_started=$(date +%s)
captured_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)

(CDPATH= cd -- "$repository" && exec "$binary" "$workspace") \
    > "$studio_stdout" 2> "$studio_stderr" &
studio_pid=$!
captured_pid=$studio_pid
kill -0 "$studio_pid" 2>/dev/null || fail "Studio process did not remain alive after launch"

if [ "$fixture" = true ]; then
    process_start=fixture-process-start
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
captured_sampler_pid=$sampler_pid

printf 'raw AX capture active for PID %s; perform only the approved Task #504 journey\n' \
    "$studio_pid"
capture_phase=ax_client
set +e
"$assurance" capture-ax-client "$studio_pid" "$generation" \
    "$pre_action_ms" "$post_action_ms" "$capture_root/raw-ax" \
    > "$ax_stdout" 2> "$ax_stderr"
ax_status=$?
set -e
if [ "$ax_status" -ne 0 ]; then
    printf 'bounded raw AX diagnostic:\n' >&2
    head -c 4096 "$ax_stderr" >&2
    printf '\n' >&2
    fail "raw AX capture command failed"
fi
for artifact_path in tree.jsonl events.jsonl latency.jsonl; do
    [ -s "$capture_root/raw-ax/$artifact_path" ] ||
        fail "raw AX capture did not publish $artifact_path"
done

capture_phase=sampler
set +e
wait "$sampler_pid"
sampler_status=$?
set -e
sampler_pid=
[ "$sampler_status" -eq 0 ] || fail "footprint sampler failed"
[ -s "$raw_footprint" ] || fail "footprint sampler did not publish machine-readable JSON"

capture_phase=residency_analysis
"$(dirname "$0")/analyze-studio-residency.sh" "$raw_footprint" \
    "$studio_pid" 0 "$capture_root/residency-analysis" \
    > "$capture_root/residency.log"

if [ "$fixture" != true ]; then
    running_binary=$(ps -ww -p "$studio_pid" -o comm= | sed 's/^[[:space:]]*//')
    [ -n "$running_binary" ] || fail "Studio exited before capture completed"
    running_binary=$(canonical_file "$running_binary") ||
        fail "post-capture executable path is unavailable"
    [ "$running_binary" = "$binary" ] || fail "Studio process identity drifted during capture"
    current_start=$(ps -ww -p "$studio_pid" -o lstart= | sed 's/^[[:space:]]*//')
    [ "$current_start" = "$process_start" ] || fail "Studio process start identity drifted"
fi

capture_phase=close
printf 'capture complete; close Alpine Studio within %s seconds\n' "$post_close_timeout"
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
[ "$studio_status" -eq 0 ] || fail "Studio process failed with status $studio_status"
capture_ended=$(date +%s)

cat > "$capture_root/workspace-record.txt" <<EOF
kind=$workspace_kind
path_sha256=$workspace_path_sha
identity_sha256=$workspace_identity_sha
revision=$workspace_revision
status_sha256=$workspace_status_sha
EOF

if [ "$fixture" = true ]; then
    os_build=fixture
    sdk_build=fixture
    hardware_model=fixture
    architecture=fixture
else
    os_build=$(sw_vers -buildVersion)
    sdk_build=$(xcrun --sdk macosx --show-sdk-build-version 2>/dev/null || printf unavailable)
    hardware_model=$(sysctl -n hw.model)
    architecture=$(uname -m)
fi

manifest="$capture_root/manifest.toml"
cat > "$manifest" <<EOF
schema = "alpine-ax-process-input/v1"
task_issue = 504
fixture_only = $fixture
evidence_scope = "$evidence_scope"
performance_claim = false
aep_0273_bundle_ready = false
repository_revision = "$revision"
repository_clean = $repository_clean
repository_status_sha256 = "$repository_status_sha"
workspace_kind = "$workspace_kind"
workspace_identity_sha256 = "$workspace_identity_sha"
studio_binary_sha256 = "$(sha256 "$binary")"
harness_binary_sha256 = "$(sha256 "$assurance")"
sampler_sha256 = "$(sha256 "$sampler")"
studio_pid = $captured_pid
process_start = "$process_start"
studio_exit_status = $studio_status
post_close_absent = true
generation = $generation
pre_action_ms = $pre_action_ms
post_action_ms = $post_action_ms
duration_seconds = $duration
interval_seconds = $interval
sample_bound = $sample_bound
started_epoch_seconds = $capture_started
ended_epoch_seconds = $capture_ended
captured_at_utc = "$captured_at"
macos_build = "$os_build"
sdk_build = "$sdk_build"
hardware_model = "$hardware_model"
architecture = "$architecture"
EOF

artifact tree raw-ax/tree.jsonl
artifact events raw-ax/events.jsonl
artifact latency raw-ax/latency.jsonl
artifact footprint footprint.json
artifact footprint_log footprint.log
artifact residency_samples residency-analysis/samples.csv
artifact residency_summary residency-analysis/summary.toml
artifact residency_log residency.log
artifact ax_stdout ax.stdout
artifact ax_stderr ax.stderr
artifact studio_stdout studio.stdout
artifact studio_stderr studio.stderr
artifact workspace_record workspace-record.txt

capture_phase=publication
[ ! -e "$output_dir" ] && [ ! -L "$output_dir" ] || fail "output directory appeared during capture"
mv "$capture_root" "$output_dir"
published=true
printf 'retained no-claim Task #504 AX process input package at %s\n' "$output_dir"
