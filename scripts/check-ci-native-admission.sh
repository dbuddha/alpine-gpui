#!/bin/bash
set -euo pipefail

# Execute the union of the native package selections once. Mutator-copy
# baselines remain enabled for explicitly requested assurance.
if [ "${CARGO_INCREMENTAL:-}" != 0 ] ||
    [ "${RUSTFLAGS:-}" != '--cfg alpine_native_validation' ] ||
    [ "${ALPINE_PRESENTATION_EVIDENCE_MODE:-}" != hosted-direct ] ||
    [ "${MACOSX_DEPLOYMENT_TARGET:-}" != 15.0 ] ||
    [ "${ALPINE_VALIDATION_DEPLOYMENT_TARGET:-}" != 26.0 ]; then
    printf 'CI native admission error: explicit hosted validation environment required\n' >&2
    exit 2
fi
unset ALPINE_RUST_ANALYZER
unset ALPINE_EDITOR_NATIVE_PROCESS_SCOPE ALPINE_EDITOR_NATIVE_ACCESSIBILITY_CHILD
unset ALPINE_EDITOR_NATIVE_ACCESSIBILITY_OMIT ALPINE_EDITOR_NATIVE_LSP_SERVER
export ALPINE_REQUIRE_NATIVE_VALIDATION=1

printf 'CI native admission: Metal toolchain\n'
if ! xcrun --sdk macosx --find metal >/dev/null 2>&1; then
    xcodebuild -downloadComponent MetalToolchain
fi
xcrun --sdk macosx --find metal
xcrun --sdk macosx --find metallib

# Do not use --all-features or a test-name filter: the failing mutator baselines
# use these package sets with default features and the native validation cfg.
printf 'CI native admission: platform and Studio default-feature tests\n'
mkdir -p target/native-acceptance
log=$(mktemp target/native-acceptance/ci-admission.XXXXXX)
if cargo test --locked --package=alpine-platform-macos --package=alpine-editor 2>&1 | tee "$log"; then
    :
else
    result=$?
    printf 'Native admission failed; retained %s\n' "$log" >&2
    exit "$result"
fi
grep -Fxq 'alpine-native-process-complete scope=all' "$log" || {
    printf 'Native admission failed: process completion receipt missing; retained %s\n' "$log" >&2
    exit 1
}
printf 'CI native admission passed; mutation-copy baselines still required\n'
