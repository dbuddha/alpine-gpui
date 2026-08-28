#!/bin/sh
set -eu

fail() {
    printf 'studio profile evidence error: %s\n' "$1" >&2
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

repo_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
package=${1:-"$repo_root/assurance/studio-profile/v1/c93950e-physical-diagnostic"}
manifest=$package/manifest.toml
[ -f "$manifest" ] || fail "manifest is missing: $manifest"
[ ! -L "$package" ] || fail 'package root must not be a symlink'

expected_files=$(cat <<'EOF'
./README.md
./analysis/counters.tsv
./analysis/omissions.tsv
./analysis/report.txt
./analysis/samples.tsv
./analysis/summary.tsv
./manifest.toml
./records.json.gz
./workload.rs
EOF
)
actual_files=$(CDPATH= cd -- "$package" && find . -type f -print | LC_ALL=C sort)
[ "$actual_files" = "$expected_files" ] || fail 'package file set is not exact'
if find "$package" -type l -print | grep -q .; then
    fail 'package files must not be symlinks'
fi

[ "$(manifest_string schema)" = 'alpine-studio-physical-profile-evidence/v1' ] ||
    fail 'unsupported package schema'
[ "$(manifest_string evidence_ceiling)" = 'diagnostic-only' ] ||
    fail 'evidence ceiling must remain diagnostic-only'
[ "$(manifest_string observer_cost_calibrated)" = 'false' ] ||
    fail 'observer cost must remain uncalibrated'
[ "$(manifest_string causal_attribution_allowed)" = 'false' ] ||
    fail 'causal attribution must remain disabled'
[ "$(manifest_string threshold_activation_allowed)" = 'false' ] ||
    fail 'threshold activation must remain disabled'
[ "$(manifest_string comparison_claim_allowed)" = 'false' ] ||
    fail 'comparison claims must remain disabled'

source_revision=$(manifest_string source_revision)
case "$source_revision" in *[!0-9a-f]*) fail 'source_revision must be lowercase hexadecimal' ;; esac
[ "${#source_revision}" -eq 40 ] || fail 'source_revision must be one Git SHA-1'
for key in executable_sha256 original_raw_sha256 workload_sha256 analyzer_sha256; do
    value=$(manifest_string "$key")
    case "$value" in *[!0-9a-f]*) fail "$key must be lowercase hexadecimal" ;; esac
    [ "${#value}" -eq 64 ] || fail "$key must be one SHA-256"
done
[ "$(hash_file "$repo_root/scripts/analyze-studio-profile.sh")" = "$(manifest_string analyzer_sha256)" ] ||
    fail 'analyzer identity changed'

check_hash records.json.gz normalized_records_gzip_sha256 'normalized gzip'
check_hash workload.rs workload_sha256 'workload'
check_hash analysis/report.txt analysis_report_sha256 'analysis report'
check_hash analysis/summary.tsv analysis_summary_sha256 'analysis summary'
check_hash analysis/samples.tsv analysis_samples_sha256 'analysis samples'
check_hash analysis/counters.tsv analysis_counters_sha256 'analysis counters'
check_hash analysis/omissions.tsv analysis_omissions_sha256 'analysis omissions'

scratch=$(mktemp -d "${TMPDIR:-/tmp}/alpine-profile-evidence.XXXXXX")
cleanup() { rm -rf "$scratch"; }
trap cleanup EXIT HUP INT TERM
gzip -t "$package/records.json.gz" || fail 'normalized records gzip is corrupt'
gzip -dc "$package/records.json.gz" > "$scratch/records.json"
[ "$(wc -c < "$scratch/records.json" | tr -d ' ')" = "$(manifest_integer normalized_records_bytes)" ] ||
    fail 'normalized records byte count mismatch'
[ "$(hash_file "$scratch/records.json")" = "$(manifest_string normalized_records_sha256)" ] ||
    fail 'normalized records hash mismatch'

record_count=$(manifest_integer normalized_record_count)
process_id=$(manifest_integer process_id)
sender_uuid=$(manifest_string sender_image_uuid)
boot_uuid=$(manifest_string normalized_boot_uuid)
process_path=$(manifest_string normalized_process_image_path)
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
' "$scratch/records.json" >/dev/null || fail 'normalized records failed privacy and shape contract'
if LC_ALL=C grep -Eiq 'deepak|/Users/|password|secret|token|workload\.rs' "$scratch/records.json"; then
    fail 'normalized records contain a private or content-bearing string'
fi

"$repo_root/scripts/analyze-studio-profile.sh" \
    "$scratch/records.json" "$scratch/analysis" \
    "$(manifest_integer mach_timebase_numer)" \
    "$(manifest_integer mach_timebase_denom)" >/dev/null
for name in report.txt summary.tsv samples.tsv counters.tsv omissions.tsv; do
    cmp "$scratch/analysis/$name" "$package/analysis/$name" >/dev/null ||
        fail "reanalysis output drift: $name"
done
grep -Fxq 'presented_sample_count=0' "$package/analysis/report.txt" ||
    fail 'retained report must preserve the missing presented-handler endpoint'
for line in \
    'observer_cost_calibrated=false' \
    'causal_attribution_allowed=false' \
    'threshold_activation_allowed=false' \
    'comparison_claim_allowed=false'; do
    grep -Fxq "$line" "$package/analysis/report.txt" ||
        fail "retained report claim boundary changed: $line"
done

printf 'retained Studio physical profile evidence is valid\n'
