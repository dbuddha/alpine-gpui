#!/bin/sh
set -eu

# Keep these existing obligations ahead of provisioning and assurance fan-out.
# Package narrowing requires the separately accepted dependency graph.
printf 'CI fast feedback: formatting\n'
cargo fmt --all -- --check
printf 'CI fast feedback: workspace Clippy\n'
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
printf 'CI fast feedback passed\n'
