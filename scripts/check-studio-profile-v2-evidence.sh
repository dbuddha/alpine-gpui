#!/bin/sh
set -eu

fail() {
    printf 'studio profile v2 evidence error: %s\n' "$1" >&2
    exit 1
}

hash_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        fail 'sha256sum or shasum is required'
    fi
}

manifest_string() {
    key=$1
    value=$(awk -F ' = ' -v key="$key" '
        $1 == key { count += 1; value = $2 }
        END { if (count != 1) exit 1; print value }
    ' "$manifest") || fail "manifest string is missing or duplicated: $key"
    case "$value" in
        \"*\") value=${value#\"}; value=${value%\"} ;;
        *) fail "manifest string is malformed: $key" ;;
    esac
    printf '%s' "$value"
}

manifest_integer() {
    key=$1
    value=$(awk -F ' = ' -v key="$key" '
        $1 == key { count += 1; value = $2 }
        END { if (count != 1) exit 1; print value }
    ' "$manifest") || fail "manifest integer is missing or duplicated: $key"
    case "$value" in ''|*[!0-9]*) fail "manifest integer is malformed: $key" ;; esac
    printf '%s' "$value"
}

check_hash() {
    relative=$1
    key=$2
    label=$3
    expected=$(manifest_string "$key")
    actual=$(hash_file "$package/$relative")
    [ "$actual" = "$expected" ] || fail "$label hash mismatch"
}

summary_value() {
    file=$1
    metric=$2
    column=$3
    value=$(awk -F '\t' -v metric="$metric" -v column="$column" '
        $1 == metric { count += 1; value = $column }
        END { if (count != 1) exit 1; print value }
    ' "$file") || fail "summary metric is missing or duplicated: $metric"
    case "$value" in ''|*[!0-9]*) fail "summary metric is malformed: $metric" ;; esac
    printf '%s' "$value"
}

repo_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
package=${1:-"$repo_root/assurance/studio-profile/v2/553-command-buffer-presentation"}
manifest=$package/manifest.toml
[ -f "$manifest" ] || fail "manifest is missing: $manifest"
[ ! -L "$package" ] || fail 'package root must not be a symlink'

expected_files=$(cat <<'EOF'
./README.md
./baseline/analysis/counters.tsv
./baseline/analysis/omissions.tsv
./baseline/analysis/report.txt
./baseline/analysis/samples.tsv
./baseline/analysis/summary.tsv
./baseline/records.json.gz
./candidate/analysis/counters.tsv
./candidate/analysis/omissions.tsv
./candidate/analysis/report.txt
./candidate/analysis/samples.tsv
./candidate/analysis/summary.tsv
./candidate/records.json.gz
./comparison.tsv
./manifest.toml
./workload.txt
EOF
)
actual_files=$(CDPATH= cd -- "$package" && find . -type f -print | LC_ALL=C sort)
[ "$actual_files" = "$expected_files" ] || fail 'package file set is not exact'
if find "$package" -type l -print | grep -q .; then
    fail 'package files must not be symlinks'
fi

[ "$(manifest_string schema)" = 'alpine-studio-physical-profile-evidence/v2' ] ||
    fail 'unsupported package schema'
[ "$(manifest_integer experiment_issue)" = 553 ] || fail 'experiment issue drifted'
[ "$(manifest_string outcome)" = 'reject-no-material-improvement' ] ||
    fail 'experiment outcome drifted'
[ "$(manifest_string evidence_ceiling)" = 'paired-e3-diagnostic-only' ] ||
    fail 'evidence ceiling drifted'
for key in observer_cost_calibrated causal_attribution_allowed \
    threshold_activation_allowed comparison_claim_allowed \
    upstream_comparison_claim_allowed; do
    [ "$(manifest_string "$key")" = false ] || fail "$key must remain false"
done

for key in baseline_revision candidate_revision; do
    value=$(manifest_string "$key")
    case "$value" in *[!0-9a-f]*) fail "$key must be lowercase hexadecimal" ;; esac
    [ "${#value}" -eq 40 ] || fail "$key must be one Git SHA-1"
done
for key in baseline_executable_sha256 candidate_executable_sha256 \
    baseline_original_raw_sha256 candidate_original_raw_sha256; do
    value=$(manifest_string "$key")
    case "$value" in *[!0-9a-f]*) fail "$key must be lowercase hexadecimal" ;; esac
    [ "${#value}" -eq 64 ] || fail "$key must be one SHA-256"
done

[ "$(hash_file "$repo_root/scripts/analyze-studio-profile-v2.sh")" = \
    "$(manifest_string analyzer_sha256)" ] || fail 'analyzer identity changed'
check_hash workload.txt workload_sha256 'workload'
check_hash comparison.tsv comparison_sha256 'comparison'
[ "$(manifest_integer workload_bytes)" = 55 ] || fail 'workload byte contract drifted'
[ "$(wc -c < "$package/workload.txt" | tr -d ' ')" = 55 ] ||
    fail 'workload byte count mismatch'
[ "$(cat "$package/workload.txt")" = \
    'command-buffer-present-553-abcdefghijklmnopqrstuvwxyz12' ] ||
    fail 'workload content drifted'

scratch=$(mktemp -d "${TMPDIR:-/tmp}/alpine-profile-v2-evidence.XXXXXX")
cleanup() { rm -rf "$scratch"; }
trap cleanup EXIT HUP INT TERM

for variant in baseline candidate; do
    records=$package/$variant/records.json.gz
    check_hash "$variant/records.json.gz" \
        "${variant}_normalized_records_gzip_sha256" "$variant normalized gzip"
    for name in report summary samples counters omissions; do
        extension=tsv
        [ "$name" = report ] && extension=txt
        check_hash "$variant/analysis/$name.$extension" \
            "${variant}_analysis_${name}_sha256" "$variant analysis $name"
    done

    gzip -t "$records" || fail "$variant normalized records gzip is corrupt"
    gzip -dc "$records" > "$scratch/$variant-records.json"
    [ "$(wc -c < "$scratch/$variant-records.json" | tr -d ' ')" = \
        "$(manifest_integer "${variant}_normalized_records_bytes")" ] ||
        fail "$variant normalized record byte count mismatch"
    [ "$(hash_file "$scratch/$variant-records.json")" = \
        "$(manifest_string "${variant}_normalized_records_sha256")" ] ||
        fail "$variant normalized record hash mismatch"

    record_count=$(manifest_integer "${variant}_normalized_record_count")
    process_id=$(manifest_integer "${variant}_process_id")
    sender_uuid=$(manifest_string "${variant}_sender_image_uuid")
    boot_uuid=$(manifest_string "${variant}_normalized_boot_uuid")
    process_path=$(manifest_string "${variant}_normalized_process_image_path")
    jq -e \
        --argjson record_count "$record_count" \
        --argjson process_id "$process_id" \
        --arg sender_uuid "$sender_uuid" \
        --arg boot_uuid "$boot_uuid" \
        --arg process_path "$process_path" '
        type == "array"
        and length == $record_count
        and all(.[];
            keys == [
                "bootUUID", "category", "eventMessage", "eventType", "formatString",
                "machTimestamp", "messageType", "processID", "processImagePath",
                "senderImageUUID", "subsystem"
            ]
            and .bootUUID == $boot_uuid
            and .category == "PersistedProfile"
            and .eventType == "logEvent"
            and .messageType == "Default"
            and .processID == $process_id
            and .processImagePath == $process_path
            and .senderImageUUID == $sender_uuid
            and .subsystem == "com.dbuddha.alpine-studio"
            and (.machTimestamp | type) == "number"
            and .machTimestamp >= 0
            and (.eventMessage | test(
                "^stage=[A-Za-z ]+ correlation=[0-9]+ event=[0-9]+ scene=[0-9]+ document=[0-9]+ buffer=[0-9]+ a=[0-9]+ b=[0-9]+ c=[0-9]+$"
            ))
        )
    ' "$scratch/$variant-records.json" >/dev/null ||
        fail "$variant normalized records failed privacy and shape contract"
    if LC_ALL=C grep -Eiq 'deepak|/Users/|password|secret|token|workload\.txt' \
        "$scratch/$variant-records.json"; then
        fail "$variant normalized records contain a private or content-bearing string"
    fi

    "$repo_root/scripts/analyze-studio-profile-v2.sh" \
        "$scratch/$variant-records.json" "$scratch/$variant-analysis" \
        "$(manifest_integer mach_timebase_numer)" \
        "$(manifest_integer mach_timebase_denom)" >/dev/null
    for name in report.txt summary.tsv samples.tsv counters.tsv omissions.tsv; do
        cmp "$scratch/$variant-analysis/$name" "$package/$variant/analysis/$name" >/dev/null ||
            fail "$variant reanalysis output drift: $name"
    done
    for line in \
        'observer_cost_calibrated=false' \
        'causal_attribution_allowed=false' \
        'threshold_activation_allowed=false' \
        'comparison_claim_allowed=false'; do
        grep -Fxq "$line" "$package/$variant/analysis/report.txt" ||
            fail "$variant claim boundary changed: $line"
    done
done

{
    printf 'metric\tbaseline_count\tbaseline_p50_ns\tcandidate_count\tcandidate_p50_ns\tcandidate_minus_baseline_ns\n'
    for metric in native_display_link_target native_target_presentation \
        native_actual_presentation native_presentation_callback_lag \
        native_submission frame_build; do
        baseline_count=$(summary_value "$package/baseline/analysis/summary.tsv" "$metric" 2)
        baseline_p50=$(summary_value "$package/baseline/analysis/summary.tsv" "$metric" 4)
        candidate_count=$(summary_value "$package/candidate/analysis/summary.tsv" "$metric" 2)
        candidate_p50=$(summary_value "$package/candidate/analysis/summary.tsv" "$metric" 4)
        printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$metric" "$baseline_count" \
            "$baseline_p50" "$candidate_count" "$candidate_p50" \
            "$((candidate_p50 - baseline_p50))"
    done
    baseline_display=$(summary_value "$package/baseline/analysis/summary.tsv" native_display_link_target 4)
    baseline_target=$(summary_value "$package/baseline/analysis/summary.tsv" native_target_presentation 4)
    candidate_display=$(summary_value "$package/candidate/analysis/summary.tsv" native_display_link_target 4)
    candidate_target=$(summary_value "$package/candidate/analysis/summary.tsv" native_target_presentation 4)
    printf 'display_target_to_target_presentation\t15\t%s\t14\t%s\t%s\n' \
        "$((baseline_target - baseline_display))" \
        "$((candidate_target - candidate_display))" \
        "$(((candidate_target - candidate_display) - (baseline_target - baseline_display)))"
    baseline_actual=$(summary_value "$package/baseline/analysis/summary.tsv" native_actual_presentation 4)
    candidate_actual=$(summary_value "$package/candidate/analysis/summary.tsv" native_actual_presentation 4)
    printf 'target_to_actual_presentation\t12\t%s\t11\t%s\t%s\n' \
        "$((baseline_actual - baseline_target))" \
        "$((candidate_actual - candidate_target))" \
        "$(((candidate_actual - candidate_target) - (baseline_actual - baseline_target)))"
} > "$scratch/comparison.tsv"
cmp "$scratch/comparison.tsv" "$package/comparison.tsv" >/dev/null ||
    fail 'derived comparison drifted'
grep -Fxq 'display_target_to_target_presentation	15	33333250	14	33333250	0' \
    "$package/comparison.tsv" || fail 'fixed presentation offset conclusion drifted'
[ "$(manifest_integer candidate_rss_before_kib)" -ge \
    "$(manifest_integer baseline_rss_before_kib)" ] || fail 'candidate memory observation drifted'
[ "$(manifest_integer candidate_rss_after_kib)" -ge \
    "$(manifest_integer baseline_rss_after_kib)" ] || fail 'candidate memory observation drifted'

printf 'retained Studio profile v2 paired evidence is valid\n'
