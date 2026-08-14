#!/bin/sh
set -eu

if ! cargo metadata --format-version 1 --no-deps | grep -q '"name":"alpine-metal"'; then
    printf 'metal validation error: classifier selected Metal but alpine-metal does not exist\n' >&2
    exit 1
fi

metallib_path=$(pwd)/target/metal/offscreen.metallib
scripts/verify-metal-library.sh
scripts/build-metal-shaders.sh "$metallib_path"
if ! cmp -s shaders/offscreen.metallib "$metallib_path"; then
    printf 'metal validation error: pinned compiler output differs from the checked-in library\n' >&2
    printf 'checked-in: ' >&2
    shasum -a 256 shaders/offscreen.metallib >&2
    printf 'fresh: ' >&2
    shasum -a 256 "$metallib_path" >&2
    exit 1
fi
export ALPINE_METALLIB_PATH="$metallib_path"

export MTL_DEBUG_LAYER=1
export MTL_DEBUG_LAYER_ERROR_MODE=assert
export MTL_SHADER_VALIDATION=1
export MTL_SHADER_VALIDATION_ENABLE_ERROR_REPORTING=1
export MTL_SHADER_VALIDATION_REPORT_TO_STDERR=1
export MTL_SHADER_VALIDATION_ABORT_ON_FAULT=1

cargo test --locked -p alpine-metal
ALPINE_CAPTURE_RSS=1 cargo test --locked -p alpine-metal \
    native::tests::cancellation_shutdown_and_steady_state_have_no_hidden_native_work \
    -- --exact --nocapture --test-threads=1
