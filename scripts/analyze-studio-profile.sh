#!/bin/sh
set -eu

usage() {
    cat <<'EOF'
usage: scripts/analyze-studio-profile.sh RECORDS_JSON OUTPUT_DIR TIMEBASE_NUMER TIMEBASE_DENOM

Validate one exact-process Alpine Studio persisted-profile capture and render
deterministic diagnostic samples, percentiles, counters, and omissions. The
output never activates a latency threshold or permits causal attribution.
EOF
}

fail() {
    printf 'studio profile error: %s\n' "$1" >&2
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

is_positive_integer() {
    case "$1" in
        ''|*[!0-9]*|0) return 1 ;;
        *) return 0 ;;
    esac
}

[ "$#" -eq 4 ] || {
    usage >&2
    exit 2
}

records=$1
output=$2
numer=$3
denom=$4

[ -f "$records" ] || fail "records file is missing: $records"
[ ! -L "$records" ] || fail "records file must not be a symlink: $records"
[ ! -e "$output" ] || fail "output already exists: $output"
is_positive_integer "$numer" || fail 'timebase numerator must be a positive integer'
is_positive_integer "$denom" || fail 'timebase denominator must be a positive integer'

record_bytes=$(wc -c < "$records" | tr -d ' ')
[ "$record_bytes" -le 67108864 ] || fail 'records file exceeds the 64 MiB ceiling'

parent=$(dirname "$output")
mkdir -p "$parent"
staging=$(mktemp -d "$parent/.studio-profile.XXXXXX")
cleanup() {
    if [ -n "${staging-}" ] && [ -d "$staging" ]; then
        rm -rf "$staging"
    fi
}
trap cleanup EXIT HUP INT TERM

parsed=$staging/parsed.tsv
jq -er '
    def known_stage($stage):
        [
            "Event Dispatch Begin",
            "State Mutation Complete",
            "Frame Build Begin",
            "Visible Layout Begin",
            "Visible Layout Complete",
            "Text Summary",
            "Layout Cache Summary",
            "Glyph Atlas Summary",
            "Atlas Publication Begin",
            "Atlas Publication Complete",
            "Atlas Publication Failed",
            "Frame Build Complete",
            "Frame Build Failed",
            "Native Event Handler Latency",
            "Native Frame Queue Latency",
            "Native Submission Latency",
            "Native GPU Terminal Observed Latency",
            "Native Presented Handler Latency",
            "Native Terminal Record Latency"
        ] | index($stage) != null;
    def expected_format($stage):
        "stage=" + $stage
        + " correlation=%{public}llu event=%{public}llu scene=%{public}llu"
        + " document=%{public}llu buffer=%{public}llu a=%{public}llu"
        + " b=%{public}llu c=%{public}llu";
    def message_pattern:
        "^stage=(?<stage>.+) correlation=(?<correlation>[0-9]+)"
        + " event=(?<event>[0-9]+) scene=(?<scene>[0-9]+)"
        + " document=(?<document>[0-9]+) buffer=(?<buffer>[0-9]+)"
        + " a=(?<a>[0-9]+) b=(?<b>[0-9]+) c=(?<c>[0-9]+)$";
    if type != "array" then error("records root must be an array")
    elif length == 0 then error("records array must not be empty")
    elif length > 500000 then error("records array exceeds the 500000 record ceiling")
    else .[] end
    | . as $record
    | if (($record.processID | type) == "number"
          and $record.processID > 0
          and ($record.processID | floor) == $record.processID)
      then . else error("processID must be a positive integer") end
    | if (($record.machTimestamp | type) == "number"
          and $record.machTimestamp >= 0
          and ($record.machTimestamp | floor) == $record.machTimestamp)
      then . else error("machTimestamp must be a nonnegative integer") end
    | if ($record.subsystem == "com.dbuddha.alpine-studio"
          and $record.category == "PersistedProfile"
          and $record.messageType == "Default"
          and $record.eventType == "logEvent")
      then . else error("record route identity is invalid") end
    | if (($record.senderImageUUID | type) == "string" and ($record.senderImageUUID | length) > 0
          and ($record.bootUUID | type) == "string" and ($record.bootUUID | length) > 0
          and ($record.processImagePath | type) == "string"
          and ($record.processImagePath | length) > 0
          and ($record.processImagePath | test("[\\t\\r\\n]") | not))
      then . else error("record process identity is invalid") end
    | if (($record.eventMessage | type) == "string"
          and ($record.eventMessage | test(message_pattern)))
      then . else error("eventMessage does not match the static profile grammar") end
    | ($record.eventMessage | capture(message_pattern)) as $message
    | if known_stage($message.stage)
      then . else error("unknown profile stage: " + $message.stage) end
    | if $record.formatString == expected_format($message.stage)
      then . else error("formatString does not match the static stage grammar") end
    | [
        ($record.machTimestamp | tostring),
        ($record.processID | tostring),
        $record.senderImageUUID,
        $record.bootUUID,
        $record.processImagePath,
        $message.stage,
        $message.correlation,
        $message.event,
        $message.scene,
        $message.document,
        $message.buffer,
        $message.a,
        $message.b,
        $message.c
      ] | @tsv
' "$records" > "$parsed" || fail 'records JSON failed structural validation'

raw_omissions=$staging/raw-omissions.tsv
awk -F '\t' '
    function reject(message) {
        print "studio profile error: " message > "/dev/stderr"
        exit 1
    }
    function is_native(stage) { return index(stage, "Native ") == 1 }
    function is_event_stage(stage) {
        return stage == "Event Dispatch Begin" || stage == "State Mutation Complete"
    }
    NR == 1 {
        process_id = $2
        sender_uuid = $3
        boot_uuid = $4
        process_path = $5
        prior_mach = $1
    }
    {
        if ($2 != process_id || $3 != sender_uuid || $4 != boot_uuid || $5 != process_path)
            reject("process identity changed inside the capture")
        if ($1 < prior_mach)
            reject("machTimestamp decreased inside the capture")
        prior_mach = $1
        identity = "x" $7 SUBSEP $6
        if (++seen[identity] != 1)
            reject("duplicate stage for one correlation: " $6)
        if ($8 != "0" && ("x" $7) != ("x" $8))
            reject("nonzero event does not match correlation")
        if (is_native($6) && ($9 != "0" || $10 != "0" || $11 != "0"))
            reject("native stage carries nonzero revision identity")
        if (is_event_stage($6) && $9 != "0")
            reject("event stage carries a nonzero scene revision")
        if (!is_native($6) && !is_event_stage($6) && $9 == "0")
            reject("frame stage carries a zero scene revision")
    }
' "$parsed"

raw_samples=$staging/raw-samples.tsv
awk -F '\t' -v numer="$numer" -v denom="$denom" '
    function ns(start, finish) { return int(((finish - start) * numer) / denom) }
    function native_metric(stage) {
        if (stage == "Native Event Handler Latency") return "native_event_handler"
        if (stage == "Native Frame Queue Latency") return "native_frame_queue"
        if (stage == "Native Submission Latency") return "native_submission"
        if (stage == "Native GPU Terminal Observed Latency") return "native_gpu_terminal_observed"
        if (stage == "Native Presented Handler Latency") return "native_presented_handler"
        if (stage == "Native Terminal Record Latency") return "native_terminal_record"
        return ""
    }
    {
        key = "x" $7
        event[key] = $8
        stage = $6
        metric = native_metric(stage)
        if (metric != "") print metric "\t" $8 "\t" $12
        if (stage == "Event Dispatch Begin") dispatch[key] = $1
        else if (stage == "State Mutation Complete") state[key] = $1
        else if (stage == "Frame Build Begin") frame_begin[key] = $1
        else if (stage == "Visible Layout Begin") layout_begin[key] = $1
        else if (stage == "Visible Layout Complete") layout_complete[key] = $1
        else if (stage == "Atlas Publication Begin") atlas_begin[key] = $1
        else if (stage == "Atlas Publication Complete") atlas_complete[key] = $1
        else if (stage == "Frame Build Complete") frame_complete[key] = $1
    }
    END {
        for (key in state)
            if (key in dispatch) print "state_mutation\t" event[key] "\t" ns(dispatch[key], state[key])
        for (key in frame_complete) {
            if (key in frame_begin) print "frame_build\t" event[key] "\t" ns(frame_begin[key], frame_complete[key])
            if (key in dispatch) print "event_to_frame_complete\t" event[key] "\t" ns(dispatch[key], frame_complete[key])
        }
        for (key in layout_complete)
            if (key in layout_begin) print "visible_layout\t" event[key] "\t" ns(layout_begin[key], layout_complete[key])
        for (key in atlas_complete)
            if (key in atlas_begin) print "atlas_publication\t" event[key] "\t" ns(atlas_begin[key], atlas_complete[key])
    }
' "$parsed" > "$raw_samples"

tab=$(printf '\t')
{
    printf 'metric\tevent\tduration_ns\n'
    sort -t "$tab" -k1,1 -k2,2n -k3,3n "$raw_samples"
} > "$staging/samples.tsv"

sort -t "$tab" -k1,1 -k3,3n "$raw_samples" | awk -F '\t' '
    function clear_values(   cursor) {
        for (cursor in values) delete values[cursor]
    }
    function emit(   p50, p95, p99) {
        if (count == 0) return
        p50 = int((50 * count + 99) / 100)
        p95 = int((95 * count + 99) / 100)
        p99 = int((99 * count + 99) / 100)
        print current "\t" count "\t" values[1] "\t" values[p50] "\t" values[p95] "\t" values[p99] "\t" values[count]
    }
    BEGIN { print "metric\tcount\tmin_ns\tp50_ns\tp95_ns\tp99_ns\tmax_ns" }
    {
        if ($1 != current) {
            emit()
            clear_values()
            current = $1
            count = 0
        }
        values[++count] = $3
    }
    END { emit() }
' > "$staging/summary.tsv"

{
    printf 'stage\tevent\tscene\tdocument\tbuffer\ta\tb\tc\n'
    awk -F '\t' '{print $6 "\t" $8 "\t" $9 "\t" $10 "\t" $11 "\t" $12 "\t" $13 "\t" $14}' "$parsed" \
        | sort -t "$tab" -k2,2n -k1,1
} > "$staging/counters.tsv"

awk -F '\t' '
    {
        key = "x" $7
        event[key] = $8
        seen[key SUBSEP $6] = 1
        if ($6 == "State Mutation Complete" && $12 == "1") visual[key] = 1
    }
    END {
        split("Frame Build Begin|Visible Layout Begin|Visible Layout Complete|Text Summary|Layout Cache Summary|Glyph Atlas Summary|Frame Build Complete|Native Event Handler Latency|Native Frame Queue Latency|Native Submission Latency|Native GPU Terminal Observed Latency|Native Presented Handler Latency|Native Terminal Record Latency", required, "|")
        for (key in visual)
            for (cursor in required)
                if (!seen[key SUBSEP required[cursor]])
                    print event[key] "\t" required[cursor]
    }
' "$parsed" > "$raw_omissions"
sort -t "$tab" -k1,1n -k2,2 "$raw_omissions" > "$staging/omissions-body.tsv"
{
    printf 'event\tmissing_stage\n'
    cat "$staging/omissions-body.tsv"
} > "$staging/omissions.tsv"

record_count=$(wc -l < "$parsed" | tr -d ' ')
event_count=$(awk -F '\t' '$8 != "0" {print $8}' "$parsed" | sort -u | wc -l | tr -d ' ')
stage_count=$(awk -F '\t' '{print $6}' "$parsed" | sort -u | wc -l | tr -d ' ')
process_id=$(awk -F '\t' 'NR == 1 {print $2}' "$parsed")
sender_uuid=$(awk -F '\t' 'NR == 1 {print $3}' "$parsed")
boot_uuid=$(awk -F '\t' 'NR == 1 {print $4}' "$parsed")
process_path=$(awk -F '\t' 'NR == 1 {print $5}' "$parsed")
presented_samples=$(awk -F '\t' '$1 == "native_presented_handler" {count++} END {print count + 0}' "$raw_samples")
omission_count=$(awk 'NR > 1 {count++} END {print count + 0}' "$staging/omissions.tsv")

cat > "$staging/report.txt" <<EOF
schema=alpine-studio-profile-analysis/v1
records_sha256=$(hash_file "$records")
records_bytes=$record_bytes
record_count=$record_count
event_count=$event_count
distinct_stage_count=$stage_count
process_id=$process_id
sender_image_uuid=$sender_uuid
boot_uuid=$boot_uuid
process_image_path=$process_path
mach_timebase_numer=$numer
mach_timebase_denom=$denom
presented_sample_count=$presented_samples
omission_count=$omission_count
observer_cost_calibrated=false
causal_attribution_allowed=false
threshold_activation_allowed=false
comparison_claim_allowed=false
EOF

mv "$staging" "$output"
staging=
printf '%s\n' "$output"
