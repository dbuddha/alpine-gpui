#!/bin/sh
set -eu

# Mirrors the CI quality job step for step, plus the native_process check.
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
if [ "$(uname -s)" = Darwin ] && [ "$(uname -m)" = arm64 ]; then
    RUSTFLAGS="${RUSTFLAGS-} --cfg alpine_native_validation" \
        cargo check --locked -p alpine-editor --test native_process
fi
scripts/check-policy.sh
scripts/check-product-boundary.sh
cargo deny check bans licenses sources advisories
cargo test --workspace --all-targets --all-features --locked
cargo test --locked -p alpine-metal --all-targets
scripts/test-studio-concurrency-stress.sh
cargo test --workspace --doc --all-features --locked
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps --locked
