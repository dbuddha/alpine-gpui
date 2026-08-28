#!/bin/sh
set -eu

if ! cargo metadata --format-version 1 --no-deps | grep -q '"name":"alpine-metal"'; then
    printf 'metal validation error: classifier selected Metal but alpine-metal does not exist\n' >&2
    exit 1
fi

metallib_path=$(pwd)/target/metal/offscreen.metallib
scripts/build-metal-shaders.sh "$metallib_path"
scripts/verify-metal-library.sh
if ! cmp -s shaders/offscreen.metallib "$metallib_path"; then
    printf 'metal validation error: pinned compiler output differs from the checked-in library\n' >&2
    printf 'checked-in: ' >&2
    shasum -a 256 shaders/offscreen.metallib >&2
    printf 'fresh: ' >&2
    shasum -a 256 "$metallib_path" >&2
    exit 1
fi
export ALPINE_METALLIB_PATH="$metallib_path"

# Keep the pinned shader at the shipping target above, but let hosted runtime
# validation match its OS so Shader Validation loads current diagnostics.
export MACOSX_DEPLOYMENT_TARGET=${ALPINE_VALIDATION_DEPLOYMENT_TARGET:-${MACOSX_DEPLOYMENT_TARGET:-15.0}}

export MTL_DEBUG_LAYER=1
export MTL_DEBUG_LAYER_ERROR_MODE=assert
export MTL_SHADER_VALIDATION=1
export MTL_SHADER_VALIDATION_ENABLE_ERROR_REPORTING=1
export MTL_SHADER_VALIDATION_REPORT_TO_STDERR=1
export MTL_SHADER_VALIDATION_ABORT_ON_FAULT=1

mkdir -p target
xcrun swiftc -parse-as-library tools/onscreen-sdr-capture/Capture.swift \
    -o target/onscreen-sdr-capture-helper
target/onscreen-sdr-capture-helper --self-test

for validation_test in \
    native::tests::renders_discriminating_scene_once_and_matches_cpu_oracle \
    native::tests::renders_a8_glyphs_and_reuses_only_identical_atlas_storage \
    native::tests::atlas_pressure_release_preserves_in_flight_drawable_ownership
do
    cargo test --locked -p alpine-metal --all-features "$validation_test" -- \
        --exact --nocapture --test-threads=1
done

cargo test --locked -p alpine-metal --all-features -- --test-threads=1
RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
    cargo test --locked -p alpine-platform-macos --test native_initialization
RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
    cargo test --locked -p alpine-platform-macos --test native_presentation
RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
    cargo test --locked -p alpine-platform-macos --test native_surface_epochs
RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
    cargo test --locked -p alpine-platform-macos --test native_color
ALPINE_PRESENTATION_EVIDENCE_MODE=hosted-direct \
    RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
    cargo test --locked -p alpine-platform-macos --test native_recovery
RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
    cargo test --locked -p alpine-platform-macos --test native_lifecycle
RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
    cargo test --locked -p alpine-platform-macos --test native_input
RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
    cargo test --locked -p alpine-platform-macos --test native_accessibility
RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
    cargo test --locked -p alpine-platform-macos --test native_runtime
ALPINE_PRESENTATION_EVIDENCE_MODE=hosted-direct \
    RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
    cargo test --locked -p alpine-platform-macos --test native_onscreen_sdr
/usr/bin/env -u ALPINE_RUST_ANALYZER \
    RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
    cargo test --locked -p alpine-studio --test native_process
/usr/bin/env \
    -u MTL_DEBUG_LAYER \
    -u MTL_DEBUG_LAYER_ERROR_MODE \
    -u MTL_SHADER_VALIDATION \
    -u MTL_SHADER_VALIDATION_ENABLE_ERROR_REPORTING \
    -u MTL_SHADER_VALIDATION_REPORT_TO_STDERR \
    -u MTL_SHADER_VALIDATION_ABORT_ON_FAULT \
    ALPINE_NATIVE_LIFECYCLE_CAPTURE_RSS=1 \
    ALPINE_REVISION="$(git rev-parse HEAD)" \
    ALPINE_NATIVE_LIFECYCLE_ARTIFACT="$(pwd)/target/native-lifecycle-soak.toml" \
    RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
    cargo test --locked -p alpine-platform-macos --test native_lifecycle || lifecycle_status=$?
if [[ "${lifecycle_status-0}" -ne 0 ]]; then
    for stage in main-thread device renderer window view color-space layer display-link run-loop; do
        /usr/bin/env \
            -u MTL_DEBUG_LAYER \
            -u MTL_DEBUG_LAYER_ERROR_MODE \
            -u MTL_SHADER_VALIDATION \
            -u MTL_SHADER_VALIDATION_ENABLE_ERROR_REPORTING \
            -u MTL_SHADER_VALIDATION_REPORT_TO_STDERR \
            -u MTL_SHADER_VALIDATION_ABORT_ON_FAULT \
            ALPINE_NATIVE_LIFECYCLE_CAPTURE_RSS=1 \
            ALPINE_NATIVE_LIFECYCLE_STAGE_RSS="$stage" \
            ALPINE_REVISION="$(git rev-parse HEAD)" \
            ALPINE_NATIVE_LIFECYCLE_ARTIFACT="$(pwd)/target/native-lifecycle-stage-$stage.toml" \
            RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
            cargo test --locked -p alpine-platform-macos --test native_lifecycle
    done
    exit "$lifecycle_status"
fi
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

printf '%s\n' '==> hosted native idle-state qualification'
ALPINE_PRESENTATION_EVIDENCE_MODE=hosted-direct \
    RUSTFLAGS="${RUSTFLAGS:-} --cfg alpine_native_validation" \
    cargo test --locked --package alpine-platform-macos --test native_idle
