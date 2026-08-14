#!/bin/sh
set -eu

scripts/check-policy.sh
scripts/test-policy.sh
scripts/test-classifier.sh
scripts/test-hierarchy.sh
scripts/test-assurance.sh
scripts/test-qualification.sh
scripts/test-core-contracts.sh
scripts/test-metal-contracts.sh
scripts/verify-metal-library.sh
scripts/check-release.sh
mdbook build
mdbook test
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
cargo test --workspace --doc --all-features --locked
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps --locked
