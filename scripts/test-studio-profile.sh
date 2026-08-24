#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
scratch=$(mktemp -d "$root/target/studio-profile-test.XXXXXX")
cleanup() {
    rm -rf "$scratch"
}
trap cleanup EXIT HUP INT TERM

valid=$scratch/valid.json
jq -n '
    def format($stage):
        "stage=" + $stage
        + " correlation=%{public}llu event=%{public}llu scene=%{public}llu"
        + " document=%{public}llu buffer=%{public}llu a=%{public}llu"
        + " b=%{public}llu c=%{public}llu";
    def record($stage; $mach; $scene; $document; $buffer; $a; $b; $c):
        {
            subsystem: "com.dbuddha.alpine-studio",
            category: "PersistedProfile",
            messageType: "Default",
            eventType: "logEvent",
            processID: 42,
            machTimestamp: $mach,
            senderImageUUID: "SENDER",
            bootUUID: "BOOT",
            processImagePath: "/fixture/alpine-studio",
            formatString: format($stage),
            eventMessage: ("stage=" + $stage
                + " correlation=7 event=7 scene=" + ($scene | tostring)
                + " document=" + ($document | tostring)
                + " buffer=" + ($buffer | tostring)
                + " a=" + ($a | tostring) + " b=" + ($b | tostring)
                + " c=" + ($c | tostring))
        };
    [
        record("Event Dispatch Begin"; 100; 0; 1; 1; 1; 0; 0),
        record("State Mutation Complete"; 110; 0; 2; 2; 1; 1; 2),
        record("Frame Build Begin"; 120; 3; 2; 2; 0; 0; 0),
        record("Visible Layout Begin"; 125; 3; 2; 2; 0; 0; 0),
        record("Visible Layout Complete"; 145; 3; 2; 2; 10; 0; 0),
        record("Text Summary"; 146; 3; 2; 2; 1; 0; 10),
        record("Layout Cache Summary"; 147; 3; 2; 2; 9; 1; 1024),
        record("Glyph Atlas Summary"; 148; 3; 2; 2; 100; 0; 4096),
        record("Atlas Publication Begin"; 150; 3; 2; 2; 1; 0; 0),
        record("Atlas Publication Complete"; 160; 3; 2; 2; 2; 64; 1),
        record("Frame Build Complete"; 180; 3; 2; 2; 100; 90; 0),
        record("Native Event Handler Latency"; 190; 0; 0; 0; 10; 0; 0),
        record("Native Frame Queue Latency"; 191; 0; 0; 0; 20; 0; 0),
        record("Native Submission Latency"; 192; 0; 0; 0; 60; 0; 0),
        record("Native GPU Terminal Observed Latency"; 193; 0; 0; 0; 90; 0; 0),
        record("Native Presented Handler Latency"; 194; 0; 0; 0; 100; 0; 0),
        record("Native Terminal Record Latency"; 195; 0; 0; 0; 110; 0; 0)
    ]
' > "$valid"

output=$scratch/output
valid_stderr=$scratch/valid.stderr
if ! scripts/analyze-studio-profile.sh "$valid" "$output" 125 3 \
    >/dev/null 2>"$valid_stderr"; then
    cat "$valid_stderr" >&2
    exit 1
fi
[ ! -s "$valid_stderr" ] || {
    cat "$valid_stderr" >&2
    exit 1
}
grep -Fq 'schema=alpine-studio-profile-analysis/v1' "$output/report.txt"
grep -Fq 'record_count=17' "$output/report.txt"
grep -Fq 'presented_sample_count=1' "$output/report.txt"
grep -Fq 'omission_count=0' "$output/report.txt"
grep -Fq 'causal_attribution_allowed=false' "$output/report.txt"
awk -F '\t' '$1 == "state_mutation" && $2 == 1 && $4 == 416 {found=1} END {exit !found}' "$output/summary.tsv"
awk -F '\t' '$1 == "frame_build" && $2 == 1 && $4 == 2500 {found=1} END {exit !found}' "$output/summary.tsv"
awk -F '\t' '$1 == "native_presented_handler" && $2 == 1 && $4 == 100 {found=1} END {exit !found}' "$output/summary.tsv"

expect_failure() {
    name=$1
    expected=$2
    shift 2
    log=$scratch/$name.log
    if "$@" >"$log" 2>&1; then
        printf 'invalid studio profile unexpectedly passed: %s\n' "$name" >&2
        exit 1
    fi
    grep -Fq "$expected" "$log"
}

make_invalid() {
    name=$1
    filter=$2
    jq "$filter" "$valid" > "$scratch/$name.json"
}

make_invalid identity '.[1].processID = 43'
expect_failure identity 'process identity changed' \
    scripts/analyze-studio-profile.sh "$scratch/identity.json" "$scratch/identity-output" 125 3

make_invalid order '.[1].machTimestamp = 99'
expect_failure order 'machTimestamp decreased' \
    scripts/analyze-studio-profile.sh "$scratch/order.json" "$scratch/order-output" 125 3

make_invalid correlation '.[1].eventMessage |= sub("correlation=7"; "correlation=8")'
expect_failure correlation 'nonzero event does not match correlation' \
    scripts/analyze-studio-profile.sh "$scratch/correlation.json" "$scratch/correlation-output" 125 3

make_invalid native-revision '.[11].eventMessage |= sub("scene=0"; "scene=1")'
expect_failure native-revision 'native stage carries nonzero revision identity' \
    scripts/analyze-studio-profile.sh "$scratch/native-revision.json" "$scratch/native-revision-output" 125 3

make_invalid duplicate '. + [(. [0] | .machTimestamp = 196)]'
expect_failure duplicate 'duplicate stage for one correlation' \
    scripts/analyze-studio-profile.sh "$scratch/duplicate.json" "$scratch/duplicate-output" 125 3

make_invalid unknown '.[3].eventMessage |= sub("Visible Layout Begin"; "Unknown Stage") | .[3].formatString |= sub("Visible Layout Begin"; "Unknown Stage")'
expect_failure unknown 'unknown profile stage' \
    scripts/analyze-studio-profile.sh "$scratch/unknown.json" "$scratch/unknown-output" 125 3

make_invalid grammar '.[3].eventMessage += " trailing"'
expect_failure grammar 'eventMessage does not match' \
    scripts/analyze-studio-profile.sh "$scratch/grammar.json" "$scratch/grammar-output" 125 3

make_invalid format '.[3].formatString = "wrong"'
expect_failure format 'formatString does not match' \
    scripts/analyze-studio-profile.sh "$scratch/format.json" "$scratch/format-output" 125 3

expect_failure timebase 'timebase numerator must be a positive integer' \
    scripts/analyze-studio-profile.sh "$valid" "$scratch/timebase-output" 0 3
expect_failure existing-output 'output already exists' \
    scripts/analyze-studio-profile.sh "$valid" "$output" 125 3

printf 'Studio profile analyzer tests passed\n'
