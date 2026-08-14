#!/bin/sh
set -eu

if ! cargo metadata --format-version 1 --no-deps | grep -q '"name":"alpine-metal"'; then
    printf 'metal validation error: classifier selected Metal but alpine-metal does not exist\n' >&2
    exit 1
fi

scripts/build-metal-shaders.sh target/metal/offscreen.metallib

export MTL_DEBUG_LAYER=1
export MTL_DEBUG_LAYER_ERROR_MODE=assert
export MTL_SHADER_VALIDATION=1
export MTL_SHADER_VALIDATION_ENABLE_ERROR_REPORTING=1
export MTL_SHADER_VALIDATION_REPORT_TO_STDERR=1
export MTL_SHADER_VALIDATION_ABORT_ON_FAULT=1

cargo test --locked -p alpine-metal
