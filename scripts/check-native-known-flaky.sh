#!/bin/sh
set -eu

# Known intermittent native failures run here, outside the required gate,
# until fixed: the native_wake teardown crash (#533) and the accessibility
# frame deadline in native_process (#622). Same validation env as check-metal.

export MACOSX_DEPLOYMENT_TARGET=${ALPINE_VALIDATION_DEPLOYMENT_TARGET:-${MACOSX_DEPLOYMENT_TARGET:-15.0}}
export MTL_DEBUG_LAYER=1
export MTL_DEBUG_LAYER_ERROR_MODE=assert
export MTL_SHADER_VALIDATION=1
export MTL_SHADER_VALIDATION_ENABLE_ERROR_REPORTING=1
export MTL_SHADER_VALIDATION_REPORT_TO_STDERR=1
export MTL_SHADER_VALIDATION_ABORT_ON_FAULT=1
export ALPINE_REQUIRE_NATIVE_VALIDATION=1
export RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation"

wake_failures=0
for iteration in $(seq 1 25); do
    if ! cargo test --locked -p alpine-platform-macos --test native_wake; then
        wake_failures=$((wake_failures + 1))
    fi
done
printf 'native_wake teardown failures: %s of 25\n' "$wake_failures"

process_status=0
/usr/bin/env -u ALPINE_RUST_ANALYZER \
    cargo test --locked -p alpine-editor --test native_process || process_status=$?
printf 'native_process under validation exit status: %s\n' "$process_status"

[ "$wake_failures" -eq 0 ] && [ "$process_status" -eq 0 ]
