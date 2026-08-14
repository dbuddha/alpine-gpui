#!/bin/sh
set -eu

cargo test --locked -p alpine-metal --all-targets
