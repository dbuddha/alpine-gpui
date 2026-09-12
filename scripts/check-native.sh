#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
cd "$repo_root"
mode=${1:-}
scope=${2:-all}
[ "$#" -ge 1 ] && [ "$#" -le 2 ] || {
    echo 'usage: scripts/check-native.sh physical|hosted [shipping|all]' >&2; exit 2;
}
case "$mode" in physical|hosted) ;; *) echo 'invalid native evidence mode' >&2; exit 2 ;; esac
case "$scope" in shipping|all) ;; *) echo 'invalid native scope' >&2; exit 2 ;; esac
[ "$(uname -s)" = Darwin ] && [ "$(uname -m)" = arm64 ] || {
    echo 'native acceptance requires Apple Silicon macOS' >&2; exit 2;
}
[ -z "${CARGO_ENCODED_RUSTFLAGS:-}" ] && [ -z "${CARGO_BUILD_TARGET:-}" ] || {
    echo 'unset CARGO_ENCODED_RUSTFLAGS and CARGO_BUILD_TARGET for native acceptance' >&2; exit 2;
}
# Prevent inherited child/scoping controls from selecting a different journey.
unset ALPINE_STUDIO_NATIVE_ACCESSIBILITY_CHILD ALPINE_STUDIO_NATIVE_ACCESSIBILITY_OMIT
unset ALPINE_STUDIO_NATIVE_LSP_SERVER ALPINE_STUDIO_NATIVE_PROCESS_SCOPE
unset ALPINE_PRESENTATION_EVIDENCE_MODE
if [ "$mode" = hosted ]; then
    export ALPINE_PRESENTATION_EVIDENCE_MODE=hosted-direct
fi
if [ "$scope" = shipping ]; then
    export ALPINE_STUDIO_NATIVE_PROCESS_SCOPE=shipping
fi
export ALPINE_REQUIRE_NATIVE_VALIDATION=1
export RUSTFLAGS="${RUSTFLAGS:-} --cfg alpine_native_validation"
mkdir -p target/native-acceptance
log=$(mktemp "target/native-acceptance/$mode-$scope.XXXXXX")
printf 'Native acceptance: mode=%s scope=%s log=%s/%s\n' "$mode" "$scope" "$repo_root" "$log"
if [ "$scope" = shipping ]; then
    set -- --package=alpine-studio --test=native_process
else
    set -- --package=alpine-studio --package=alpine-platform-macos --package=alpine-metal --package=alpine-runtime --all-targets
fi
if cargo test --locked "$@" >"$log" 2>&1; then
    cat "$log"
else
    result=$?
    cat "$log" >&2
    printf 'Native acceptance failed (%s); retained %s/%s\n' "$result" "$repo_root" "$log" >&2
    exit "$result"
fi
grep -Fxq "alpine-native-process-complete scope=$scope" "$log" || {
    echo 'native acceptance failed: Studio completion receipt missing' >&2; exit 1;
}
printf 'Native acceptance passed: mode=%s scope=%s\n' "$mode" "$scope"
