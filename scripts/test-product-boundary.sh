#!/bin/sh
set -eu

fixture_dir=$(mktemp -d)
trap 'rm -rf "$fixture_dir"' EXIT HUP INT TERM

dependencies=$(cat assurance/alpine-studio-dependencies.txt)

ALPINE_PRODUCT_DEPENDENCY_INPUT=$dependencies \
ALPINE_PRODUCT_SYMBOL_INPUT='_objc_msgSend' \
ALPINE_PRODUCT_STRING_INPUT='Alpine Studio' \
    scripts/check-product-boundary.sh --binary >/dev/null

invalid_dependencies=$(printf '%s\nreqwest v0.12.0\n' "$dependencies")
if ALPINE_PRODUCT_DEPENDENCY_INPUT=$invalid_dependencies \
    scripts/check-product-boundary.sh > "$fixture_dir/dependency.log" 2>&1; then
    printf 'product boundary test error: dependency widening unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'shipping dependency closure changed' "$fixture_dir/dependency.log"

if ALPINE_PRODUCT_DEPENDENCY_INPUT=$dependencies \
    ALPINE_PRODUCT_SOURCE_INPUT='use std::net::TcpStream;' \
    scripts/check-product-boundary.sh > "$fixture_dir/source.log" 2>&1; then
    printf 'product boundary test error: network source unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'shipping source contains a network capability' "$fixture_dir/source.log"

if ALPINE_PRODUCT_DEPENDENCY_INPUT=$dependencies \
    ALPINE_PRODUCT_SOURCE_INPUT='' \
    ALPINE_PRODUCT_FEATURE_INPUT='telemetry = []' \
    scripts/check-product-boundary.sh > "$fixture_dir/feature.log" 2>&1; then
    printf 'product boundary test error: excluded feature unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'shipping manifest declares an excluded product feature' "$fixture_dir/feature.log"

if ALPINE_PRODUCT_DEPENDENCY_INPUT=$dependencies \
    ALPINE_PRODUCT_SOURCE_INPUT='' \
    ALPINE_PRODUCT_FEATURE_INPUT='' \
    ALPINE_PRODUCT_PATH_INPUT='apps/alpine-studio/src/plugin_host.rs' \
    scripts/check-product-boundary.sh > "$fixture_dir/path.log" 2>&1; then
    printf 'product boundary test error: excluded subsystem path unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'shipping source path declares an excluded subsystem' "$fixture_dir/path.log"

if ALPINE_PRODUCT_DEPENDENCY_INPUT=$dependencies \
    ALPINE_PRODUCT_SOURCE_INPUT='' \
    ALPINE_PRODUCT_FEATURE_INPUT='plugins = []' \
    scripts/check-product-boundary.sh > "$fixture_dir/plural-feature.log" 2>&1; then
    printf 'product boundary test error: plural excluded feature unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'shipping manifest declares an excluded product feature' "$fixture_dir/plural-feature.log"

if ALPINE_PRODUCT_DEPENDENCY_INPUT=$dependencies \
    ALPINE_PRODUCT_SOURCE_INPUT='' \
    ALPINE_PRODUCT_FEATURE_INPUT='' \
    ALPINE_PRODUCT_PATH_INPUT='apps/alpine-studio/src/extensions/mod.rs' \
    scripts/check-product-boundary.sh > "$fixture_dir/plural-path.log" 2>&1; then
    printf 'product boundary test error: plural excluded subsystem path unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'shipping source path declares an excluded subsystem' "$fixture_dir/plural-path.log"

if ALPINE_PRODUCT_DEPENDENCY_INPUT=$dependencies \
    ALPINE_PRODUCT_SOURCE_INPUT='' \
    ALPINE_PRODUCT_FEATURE_INPUT='' \
    ALPINE_PRODUCT_PATH_INPUT='' \
    ALPINE_PRODUCT_SYMBOL_INPUT='_socket' \
    ALPINE_PRODUCT_STRING_INPUT='Alpine Studio' \
    scripts/check-product-boundary.sh --binary > "$fixture_dir/symbol.log" 2>&1; then
    printf 'product boundary test error: network symbol unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'release binary imports network symbols' "$fixture_dir/symbol.log"

if ALPINE_PRODUCT_DEPENDENCY_INPUT=$dependencies \
    ALPINE_PRODUCT_SOURCE_INPUT='' \
    ALPINE_PRODUCT_FEATURE_INPUT='' \
    ALPINE_PRODUCT_PATH_INPUT='' \
    ALPINE_PRODUCT_SYMBOL_INPUT='_objc_msgSend' \
    ALPINE_PRODUCT_STRING_INPUT='https://telemetry.invalid/v1' \
    scripts/check-product-boundary.sh --binary > "$fixture_dir/endpoint.log" 2>&1; then
    printf 'product boundary test error: endpoint string unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'release binary contains a network endpoint' "$fixture_dir/endpoint.log"

printf 'Alpine Studio product boundary tests passed\n'
