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

manifest_value() {
    key=$1
    value=$(awk -F ' = ' -v key="$key" '
        $1 == key { count += 1; value = $2 }
        END { if (count != 1) exit 1; print value }
    ' "$manifest") || fail "manifest value is missing or duplicated: $key"
    printf '%s' "$value"
}

manifest_string() {
    key=$1
    value=$(manifest_value "$key")
    case "$value" in
        \"*\") value=${value#\"}; value=${value%\"} ;;
        *) fail "manifest string is malformed: $key" ;;
    esac
    printf '%s' "$value"
}

manifest_integer() {
    key=$1
    value=$(manifest_value "$key")
    case "$value" in ''|*[!0-9]*) fail "manifest integer is malformed: $key" ;; esac
    printf '%s' "$value"
}

manifest_bool() {
    key=$1
    value=$(manifest_value "$key")
    case "$value" in true|false) ;; *) fail "manifest boolean is malformed: $key" ;; esac
    printf '%s' "$value"
}

check_hex() {
    key=$1
    length=$2
    value=$(manifest_string "$key")
    case "$value" in *[!0-9a-f]*) fail "$key must be lowercase hexadecimal" ;; esac
    [ "${#value}" -eq "$length" ] || fail "$key has the wrong length"
}

check_hash() {
    relative=$1
    key=$2
    label=$3
    [ "$(hash_file "$package/$relative")" = "$(manifest_string "$key")" ] ||
        fail "$label hash mismatch"
}

derive_presentation() {
    samples=$1
    output=$2
    awk -F '\t' '
        NR > 1 {
            if ($1 == "native_display_link_target") display[$2] = $3
            else if ($1 == "native_target_presentation") target[$2] = $3
            else if ($1 == "native_actual_presentation") actual[$2] = $3
        }
        END {
            print "event\ttarget_minus_display_link_ns\tactual_minus_target_ns"
            for (event in target)
                if (event in display && event in actual)
                    print event "\t" target[event] - display[event] "\t" actual[event] - target[event]
        }
    ' "$samples" | {
        IFS= read -r header
        printf '%s\n' "$header"
        sort -n
    } > "$output"
}

check_variant() {
    variant=$1
    prefix=$2
    expected_samples=$3
    records=$scratch/$variant-records.json
    gzip -t "$package/$variant/records.json.gz" ||
        fail "$variant records gzip is corrupt"
    gzip -dc "$package/$variant/records.json.gz" > "$records"

    [ "$(wc -c < "$records" | tr -d ' ')" = "$(manifest_integer "${prefix}_normalized_records_bytes")" ] ||
        fail "$variant normalized byte count mismatch"
    [ "$(hash_file "$records")" = "$(manifest_string "${prefix}_normalized_records_sha256")" ] ||
        fail "$variant normalized records hash mismatch"
    [ "$(hash_file "$package/$variant/records.json.gz")" = "$(manifest_string "${prefix}_normalized_records_gzip_sha256")" ] ||
        fail "$variant normalized gzip hash mismatch"

    record_count=$(manifest_integer "${prefix}_normalized_record_count")
    process_id=$(manifest_integer "${prefix}_process_id")
    sender_uuid=$(manifest_string "${prefix}_sender_image_uuid")
    jq -e \
        --argjson record_count "$record_count" \
        --argjson process_id "$process_id" \
        --arg sender_uuid "$sender_uuid" '
        type == "array"
        and length == $record_count
        and all(.[];
            keys == [
                "bootUUID", "category", "eventMessage", "eventType", "formatString",
                "machTimestamp", "messageType", "processID", "processImagePath",
                "senderImageUUID", "subsystem"
            ]
            and .bootUUID == "alpine-redacted-boot-one"
            and .category == "PersistedProfile"
            and .eventType == "logEvent"
            and .messageType == "Default"
            and .processID == $process_id
            and .processImagePath == "Alpine Studio.app/Contents/MacOS/alpine-studio"
            and .senderImageUUID == $sender_uuid
            and .subsystem == "com.dbuddha.alpine-studio"
            and (.machTimestamp | type) == "number"
            and .machTimestamp >= 0
            and (.eventMessage | test(
                "^stage=[A-Za-z ]+ correlation=[0-9]+ event=[0-9]+ scene=[0-9]+ document=[0-9]+ buffer=[0-9]+ a=[0-9]+ b=[0-9]+ c=[0-9]+$"
            ))
        )
    ' "$records" >/dev/null ||
        fail "$variant normalized records failed privacy and shape contract"
    if LC_ALL=C grep -Eiq 'deepak|/Users/|password|secret|token|typing\.rs' "$records"; then
        fail "$variant normalized records contain a private or content-bearing string"
    fi

    analysis=$scratch/$variant-analysis
    "$repo_root/scripts/analyze-studio-profile-v2.sh" \
        "$records" "$analysis" \
        "$(manifest_integer mach_timebase_numer)" \
        "$(manifest_integer mach_timebase_denom)" >/dev/null
    for name in report.txt summary.tsv samples.tsv counters.tsv omissions.tsv; do
        cmp "$analysis/$name" "$package/$variant/analysis/$name" >/dev/null ||
            fail "$variant reanalysis output drift: $name"
    done
    derive_presentation "$analysis/samples.tsv" "$scratch/$variant-presentation-derived.tsv"
    cmp "$scratch/$variant-presentation-derived.tsv" \
        "$package/$variant/analysis/presentation-derived.tsv" >/dev/null ||
        fail "$variant rederived presentation output drift"

    rows=$(awk -F '\t' -v expected=33333250 '
        NR > 1 {
            if ($2 != expected) exit 2
            count += 1
        }
        END { print count + 0 }
    ' "$package/$variant/analysis/presentation-derived.tsv") ||
        fail "$variant fabricated scheduling improvement"
    [ "$rows" -eq "$expected_samples" ] ||
        fail "$variant complete sample count changed"
    for line in \
        'observer_cost_calibrated=false' \
        'causal_attribution_allowed=false' \
        'threshold_activation_allowed=false' \
        'comparison_claim_allowed=false'; do
        grep -Fxq "$line" "$package/$variant/analysis/report.txt" ||
            fail "$variant retained report claim boundary changed: $line"
    done
}

repo_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
package=${1:-"$repo_root/assurance/studio-profile/v2/544-frame-latency-negative"}
manifest=$package/manifest.toml
[ -f "$manifest" ] || fail "manifest is missing: $manifest"
[ ! -L "$package" ] || fail 'package root must not be a symlink'

expected_files=$(cat <<'EOF'
./README.md
./manifest.toml
./one-frame/analysis/counters.tsv
./one-frame/analysis/omissions.tsv
./one-frame/analysis/presentation-derived.tsv
./one-frame/analysis/report.txt
./one-frame/analysis/samples.tsv
./one-frame/analysis/summary.tsv
./one-frame/records.json.gz
./pair-summary.tsv
./two-frame/analysis/counters.tsv
./two-frame/analysis/omissions.tsv
./two-frame/analysis/presentation-derived.tsv
./two-frame/analysis/report.txt
./two-frame/analysis/samples.tsv
./two-frame/analysis/summary.tsv
./two-frame/records.json.gz
./workload.rs
EOF
)
actual_files=$(CDPATH= cd -- "$package" && find . -type f -print | LC_ALL=C sort)
[ "$actual_files" = "$expected_files" ] || fail 'package file set is not exact'
if find "$package" -type l -print | grep -q .; then
    fail 'package files must not be symlinks'
fi

[ "$(manifest_string schema)" = 'alpine-studio-frame-latency-experiment/v2' ] ||
    fail 'unsupported package schema'
[ "$(manifest_integer issue)" = 544 ] || fail 'issue identity changed'
[ "$(manifest_integer parent_defect)" = 304 ] || fail 'parent defect changed'
[ "$(manifest_integer parent_experiment)" = 331 ] || fail 'parent experiment changed'
[ "$(manifest_integer instrumentation_task)" = 522 ] ||
    fail 'instrumentation task changed'
[ "$(manifest_integer presentation_task)" = 513 ] ||
    fail 'presentation task changed'
[ "$(manifest_string result)" = 'rejected-no-scheduling-change' ] ||
    fail 'result changed'
[ "$(manifest_string production_policy)" = 'preferredFrameLatency=2.0' ] ||
    fail 'production policy changed'
[ "$(manifest_string candidate_policy)" = 'preferredFrameLatency=1.0' ] ||
    fail 'candidate policy changed'
[ "$(manifest_string evidence_ceiling)" = 'diagnostic-only' ] ||
    fail 'evidence ceiling changed'
for key in observer_cost_calibrated causal_attribution_allowed threshold_activation_allowed comparison_claim_allowed; do
    [ "$(manifest_bool "$key")" = false ] || fail "$key must remain false"
done
for key in two_frame_original_raw_retained one_frame_original_raw_retained; do
    [ "$(manifest_bool "$key")" = false ] || fail "$key must remain false"
done

for key in two_frame_source_revision one_frame_source_revision; do
    check_hex "$key" 40
done
for key in \
    two_frame_executable_sha256 one_frame_executable_sha256 \
    initial_document_sha256 final_document_sha256 workload_sha256 \
    analyzer_sha256 two_frame_original_raw_sha256 \
    two_frame_normalized_records_sha256 two_frame_normalized_records_gzip_sha256 \
    one_frame_original_raw_sha256 one_frame_normalized_records_sha256 \
    one_frame_normalized_records_gzip_sha256 two_frame_report_sha256 \
    two_frame_summary_sha256 two_frame_samples_sha256 two_frame_counters_sha256 \
    two_frame_omissions_sha256 two_frame_presentation_derived_sha256 \
    one_frame_report_sha256 one_frame_summary_sha256 one_frame_samples_sha256 \
    one_frame_counters_sha256 one_frame_omissions_sha256 \
    one_frame_presentation_derived_sha256 pair_summary_sha256; do
    check_hex "$key" 64
done
[ "$(hash_file "$repo_root/scripts/analyze-studio-profile-v2.sh")" = "$(manifest_string analyzer_sha256)" ] ||
    fail 'version 2 analyzer identity changed'
[ "$(hash_file "$package/workload.rs")" = "$(manifest_string workload_sha256)" ] ||
    fail 'workload hash mismatch'
[ "$(manifest_string workload_sha256)" = "$(manifest_string final_document_sha256)" ] ||
    fail 'matched final document identity changed'
manifest_integer two_frame_original_raw_bytes >/dev/null
manifest_integer one_frame_original_raw_bytes >/dev/null

check_hash pair-summary.tsv pair_summary_sha256 'pair summary'
for variant in two-frame one-frame; do
    prefix=$(printf '%s' "$variant" | tr '-' '_')
    for name in report summary samples counters omissions presentation-derived; do
        key=$prefix"_$(printf '%s' "$name" | tr '-' '_')_sha256"
        extension=tsv
        [ "$name" = report ] && extension=txt
        check_hash "$variant/analysis/$name.$extension" "$key" "$variant $name"
    done
done

scratch=$(mktemp -d "${TMPDIR:-/tmp}/alpine-profile-v2-evidence.XXXXXX")
cleanup() { rm -rf "$scratch"; }
trap cleanup EXIT HUP INT TERM
check_variant two-frame two_frame 42
check_variant one-frame one_frame 44

tab=$(printf '\t')
grep -Fxq "two-frame${tab}42${tab}33333250${tab}33333250${tab}33333250${tab}33333250${tab}33333250" \
    "$package/pair-summary.tsv" || fail 'two-frame pair summary changed'
grep -Fxq "one-frame${tab}44${tab}33333250${tab}33333250${tab}33333250${tab}33333250${tab}33333250" \
    "$package/pair-summary.tsv" || fail 'one-frame pair summary changed'

printf 'retained Studio frame-latency v2 evidence is valid\n'
