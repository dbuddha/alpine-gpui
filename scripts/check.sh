#!/bin/sh
set -eu

scripts/check-policy.sh
scripts/test-policy.sh
scripts/check-product-boundary.sh
scripts/test-product-boundary.sh
scripts/check-agent-skills.sh
scripts/test-agent-skills.sh
scripts/check-research-retention.sh
scripts/test-research-retention.sh
scripts/check-wiki.sh
scripts/test-wiki.sh
scripts/test-classifier.sh
scripts/test-hierarchy.sh
scripts/test-assurance.sh
scripts/test-qualification.sh
scripts/test-zed-lab-evidence.sh
scripts/test-calibration.sh
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
