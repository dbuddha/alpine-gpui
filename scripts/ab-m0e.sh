#!/bin/sh
set -u

# Throwaway A/B on one hosted VM, never merged: arm A and arm B alternate
# native_wake and missing-close runs, and SIGSEGV cores go through lldb.

a_sha=${AB_A_SHA:?}
b_sha=${AB_B_SHA:?}
pairs=${AB_PAIRS:-300}
close_pairs=${AB_CLOSE_PAIRS:-150}
lldb_runs=${AB_LLDB_RUNS:-0}

export MACOSX_DEPLOYMENT_TARGET=${ALPINE_VALIDATION_DEPLOYMENT_TARGET:-${MACOSX_DEPLOYMENT_TARGET:-15.0}}
export MTL_DEBUG_LAYER=1
export MTL_DEBUG_LAYER_ERROR_MODE=assert
export MTL_SHADER_VALIDATION=1
export MTL_SHADER_VALIDATION_ENABLE_ERROR_REPORTING=1
export MTL_SHADER_VALIDATION_REPORT_TO_STDERR=1
export MTL_SHADER_VALIDATION_ABORT_ON_FAULT=1
export ALPINE_REQUIRE_NATIVE_VALIDATION=1
export RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation"

root=$(pwd)
out="$root/target/ab"
mkdir -p "$out/failures" "$out/backtraces"
cat >"$out/get-task-allow.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>com.apple.security.get-task-allow</key><true/></dict></plist>
PLIST
ulimit -c unlimited
sudo chmod 1777 /cores 2>/dev/null || true
sudo DevToolsSecurity -enable >/dev/null 2>&1 || true
sysctl kern.coredump kern.corefile
git fetch -q --depth=1 origin "$a_sha" "$b_sha"

for arm in a b; do
    eval "sha=\$${arm}_sha"
    git worktree add -q --detach "$root/../arm-$arm" "$sha"
    (cd "$root/../arm-$arm" &&
        cargo test --locked -p alpine-platform-macos --test native_wake \
            --test native_lifecycle --no-run 2>&1 |
        sed -n 's/.*Executable tests\/\(native_[a-z]*\)\.rs (\(.*\))/\1 \2/p' \
            >"$out/bins-$arm.txt")
    while read -r test path; do
        codesign -s - -f --entitlements "$out/get-task-allow.plist" \
            "$root/../arm-$arm/$path" 2>/dev/null
    done <"$out/bins-$arm.txt"
    cat "$out/bins-$arm.txt"
done

bin_of() {
    printf '%s/../arm-%s/%s' "$root" "$1" \
        "$(awk -v t="$2" '$1 == t {print $2}' "$out/bins-$1.txt")"
}

symbolize() {
    core=$(ls -t /cores/core.* 2>/dev/null | head -n 1)
    trace="$out/backtraces/$1-$2-$3.txt"
    if [ -n "$core" ]; then
        perl -e 'alarm 180; exec @ARGV' lldb --batch -c "$core" "$4" \
            -o "thread backtrace all" >"$trace" 2>&1
        rm -f /cores/core.*
    else
        printf 'no core file\n' >"$trace"
    fi
    head -n 80 "$trace"
}

for arm in a b; do
    for test in native_wake native_lifecycle; do
        eval "runs_${arm}_${test}=0 fails_${arm}_${test}=0 segv_${arm}_${test}=0"
    done
done

run_one() {
    arm=$1
    test=$2
    i=$3
    shift 3
    bin=$(bin_of "$arm" "$test")
    status=0
    (cd "$root/../arm-$arm/crates/alpine-platform-macos" && env "$@" "$bin") \
        >"$out/last.log" 2>&1 || status=$?
    eval "runs_${arm}_${test}=\$((runs_${arm}_${test} + 1))"
    if [ "$status" -ne 0 ]; then
        eval "fails_${arm}_${test}=\$((fails_${arm}_${test} + 1))"
        cp "$out/last.log" "$out/failures/$arm-$test-$i.log"
        printf '== arm %s %s iteration %s exit %s\n' "$arm" "$test" "$i" "$status"
        tail -n 12 "$out/last.log"
        if [ "$status" -eq 139 ]; then
            eval "segv_${arm}_${test}=\$((segv_${arm}_${test} + 1))"
            symbolize "$arm" "$test" "$i" "$bin"
        fi
    fi
}

i=1
while [ "$i" -le "$pairs" ]; do
    if [ $((i % 2)) -eq 0 ]; then first=a second=b; else first=b second=a; fi
    run_one "$first" native_wake "$i"
    run_one "$second" native_wake "$i"
    i=$((i + 1))
done
i=1
while [ "$i" -le "$close_pairs" ]; do
    if [ $((i % 2)) -eq 0 ]; then first=a second=b; else first=b second=a; fi
    run_one "$first" native_lifecycle "$i" ALPINE_NATIVE_LIFECYCLE_SCENARIO=missing-close-control
    run_one "$second" native_lifecycle "$i" ALPINE_NATIVE_LIFECYCLE_SCENARIO=missing-close-control
    i=$((i + 1))
done

# Live capture: the B arm under lldb, which prints every thread on a crash.
lldb_crashes=0
i=1
bin=$(bin_of b native_wake)
while [ "$i" -le "$lldb_runs" ]; do
    trace="$out/backtraces/lldb-b-native_wake-$i.txt"
    (cd "$root/../arm-b/crates/alpine-platform-macos" &&
        perl -e 'alarm 120; exec @ARGV' lldb --batch -o run \
            -k "thread backtrace all" -k "quit 1" -- "$bin") >"$trace" 2>&1
    if grep -q "stop reason = EXC_BAD_ACCESS\|stop reason = signal SIGSEGV" "$trace"; then
        lldb_crashes=$((lldb_crashes + 1))
        printf '== lldb run %s crashed\n' "$i"
        grep -v "^native_wake phase" "$trace" | head -n 120
    else
        rm -f "$trace"
    fi
    i=$((i + 1))
done

{
    printf 'arm\ttest\truns\tfailures\tsegv\n'
    for arm in a b; do
        for test in native_wake native_lifecycle; do
            eval "printf '%s\t%s\t%s\t%s\t%s\n' $arm $test \$runs_${arm}_${test} \$fails_${arm}_${test} \$segv_${arm}_${test}"
        done
    done
    printf 'lldb\tnative_wake\t%s\t%s\t-\n' "$lldb_runs" "$lldb_crashes"
} | tee "$out/summary.tsv"
