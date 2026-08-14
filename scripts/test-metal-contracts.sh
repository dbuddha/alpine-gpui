#!/bin/sh
set -eu

scripts/test-metal-library.sh
cargo test --locked -p alpine-metal --all-targets
