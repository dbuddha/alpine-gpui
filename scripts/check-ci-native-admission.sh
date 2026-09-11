#!/bin/sh
set -eu

# This is an early guard for the two native mutation package selections, not a
# reusable cargo-mutants baseline receipt. Mutator-copy baselines remain enabled.
if [ "${CARGO_INCREMENTAL:-}" != 0 ] ||
    [ "${RUSTFLAGS:-}" != '--cfg alpine_native_validation' ] ||
    [ "${ALPINE_PRESENTATION_EVIDENCE_MODE:-}" != hosted-direct ] ||
    [ "${MACOSX_DEPLOYMENT_TARGET:-}" != 15.0 ] ||
    [ "${ALPINE_VALIDATION_DEPLOYMENT_TARGET:-}" != 26.0 ]; then
    printf 'CI native admission error: explicit hosted validation environment required\n' >&2
    exit 2
fi
unset ALPINE_RUST_ANALYZER

printf 'CI native admission: Metal toolchain\n'
if ! xcrun --sdk macosx --find metal >/dev/null 2>&1; then
    xcodebuild -downloadComponent MetalToolchain
fi
xcrun --sdk macosx --find metal
xcrun --sdk macosx --find metallib

# Do not use --all-features or a test-name filter: the failing mutator baselines
# use these package sets with default features and the native validation cfg.
printf 'CI native admission: Studio default-feature baseline\n'
cargo test --locked --package=alpine-studio
printf 'CI native admission: platform and Studio default-feature baseline\n'
cargo test --locked --package=alpine-platform-macos --package=alpine-studio
printf 'CI native admission passed; mutation-copy baselines still required\n'
