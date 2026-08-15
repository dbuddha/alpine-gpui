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

cargo test --locked -p alpine-metal --all-features
RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
    cargo test --locked -p alpine-platform-macos --test native_initialization
RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
    cargo test --locked -p alpine-platform-macos --test native_presentation
RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
    cargo test --locked -p alpine-platform-macos --test native_surface_epochs
RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
    cargo test --locked -p alpine-platform-macos --test native_color
RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
    cargo test --locked -p alpine-platform-macos --test native_recovery
RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
    cargo test --locked -p alpine-platform-macos --test native_lifecycle
/usr/bin/env \
    -u MTL_DEBUG_LAYER \
    -u MTL_DEBUG_LAYER_ERROR_MODE \
    -u MTL_SHADER_VALIDATION \
    -u MTL_SHADER_VALIDATION_ENABLE_ERROR_REPORTING \
    -u MTL_SHADER_VALIDATION_REPORT_TO_STDERR \
    -u MTL_SHADER_VALIDATION_ABORT_ON_FAULT \
    ALPINE_CAPTURE_RSS=1 cargo test --locked -p alpine-metal \
    native::tests::cancellation_shutdown_and_steady_state_have_no_hidden_native_work \
    -- --exact --nocapture --test-threads=1
