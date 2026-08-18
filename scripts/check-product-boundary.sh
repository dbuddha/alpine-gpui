#!/bin/sh
set -eu

mode=${1:-check}
case "$mode" in
    check|--binary) ;;
    *)
        printf 'product boundary error: expected no argument or --binary\n' >&2
        exit 2
        ;;
esac

expected_dependencies=assurance/alpine-studio-dependencies.txt
if [ ! -f "$expected_dependencies" ]; then
    printf 'product boundary error: missing dependency closure %s\n' "$expected_dependencies" >&2
    exit 1
fi

temporary_dir=$(mktemp -d)
trap 'rm -rf "$temporary_dir"' EXIT HUP INT TERM
actual_dependencies="$temporary_dir/dependencies.txt"

if [ "${ALPINE_PRODUCT_DEPENDENCY_INPUT+x}" = x ]; then
    printf '%s\n' "$ALPINE_PRODUCT_DEPENDENCY_INPUT" > "$actual_dependencies"
else
    cargo tree --locked -p alpine-studio \
        --target aarch64-apple-darwin \
        --edges normal,build \
        --prefix none \
        --format '{p}' \
        | sed -E 's/ \(\*\)$//; s# \(/[^)]*\)$##' \
        | LC_ALL=C sort -u \
        > "$actual_dependencies"
fi

if ! cmp -s "$expected_dependencies" "$actual_dependencies"; then
    printf 'product boundary error: Alpine Studio shipping dependency closure changed\n' >&2
    diff -u "$expected_dependencies" "$actual_dependencies" >&2 || true
    exit 1
fi

if [ "${ALPINE_PRODUCT_SOURCE_INPUT+x}" = x ]; then
    network_source=$ALPINE_PRODUCT_SOURCE_INPUT
else
    network_source=$(git grep -n -I -E \
        'std::net|TcpStream|TcpListener|UdpSocket|UnixStream|UnixListener|https?://|wss?://' \
        -- 'apps/alpine-studio/src/*.rs' 'crates/*/src/*.rs' || true)
fi
if [ -n "$network_source" ]; then
    printf 'product boundary error: shipping source contains a network capability\n' >&2
    printf '%s\n' "$network_source" >&2
    exit 1
fi

if [ "${ALPINE_PRODUCT_FEATURE_INPUT+x}" = x ]; then
    excluded_features=$ALPINE_PRODUCT_FEATURE_INPUT
else
    excluded_features=$(find apps/alpine-studio crates -name Cargo.toml -type f -print0 \
        | xargs -0 grep -nEH \
            '^[[:space:]]*(ai|cloud|collab|collaboration|debugger|extension|marketplace|plugin|remote|telemetry)s?[[:space:]]*=' \
            2>/dev/null || true)
fi
if [ -n "$excluded_features" ]; then
    printf 'product boundary error: shipping manifest declares an excluded product feature\n' >&2
    printf '%s\n' "$excluded_features" >&2
    exit 1
fi

if [ "${ALPINE_PRODUCT_PATH_INPUT+x}" = x ]; then
    excluded_paths=$ALPINE_PRODUCT_PATH_INPUT
else
    excluded_paths=$(find apps/alpine-studio/src crates -type f \
        | grep -Ei '/(ai|cloud|collab|collaboration|debugger|extension|marketplace|plugin|remote|telemetry)s?([-_.]|/)' \
        || true)
fi
if [ -n "$excluded_paths" ]; then
    printf 'product boundary error: shipping source path declares an excluded subsystem\n' >&2
    printf '%s\n' "$excluded_paths" >&2
    exit 1
fi

if [ "$mode" = --binary ]; then
    if [ "${ALPINE_PRODUCT_SYMBOL_INPUT+x}" = x ] \
        && [ "${ALPINE_PRODUCT_STRING_INPUT+x}" = x ]; then
        binary_symbols=$ALPINE_PRODUCT_SYMBOL_INPUT
        binary_strings=$ALPINE_PRODUCT_STRING_INPUT
        binary_identity=fixture
        binary_bytes=0
    elif [ "${ALPINE_PRODUCT_SYMBOL_INPUT+x}" = x ] \
        || [ "${ALPINE_PRODUCT_STRING_INPUT+x}" = x ]; then
        printf 'product boundary error: binary fixture requires both symbol and string input\n' >&2
        exit 2
    else
        cargo build --release -p alpine-studio --locked
        binary=target/release/alpine-studio
        if [ ! -x "$binary" ]; then
            printf 'product boundary error: missing Alpine Studio release binary\n' >&2
            exit 1
        fi
        binary_symbols=$(nm -u "$binary")
        binary_strings=$(strings -a "$binary")
        binary_identity=$(shasum -a 256 "$binary" | awk '{print $1}')
        binary_bytes=$(wc -c < "$binary" | tr -d ' ')
    fi

    network_symbols=$(printf '%s\n' "$binary_symbols" \
        | grep -E '(_|[[:space:]])(accept|connect|getaddrinfo|listen|recvfrom|sendto|socket)(@|$)' \
        || true)
    if [ -n "$network_symbols" ]; then
        printf 'product boundary error: release binary imports network symbols\n' >&2
        printf '%s\n' "$network_symbols" >&2
        exit 1
    fi

    endpoint_strings=$(printf '%s\n' "$binary_strings" \
        | grep -Ei 'https?://|wss?://|api\.openai\.com|sentry\.io|telemetry endpoint' \
        || true)
    if [ -n "$endpoint_strings" ]; then
        printf 'product boundary error: release binary contains a network endpoint\n' >&2
        printf '%s\n' "$endpoint_strings" >&2
        exit 1
    fi

    printf 'binary_sha256=%s\n' "$binary_identity"
    printf 'binary_bytes=%s\n' "$binary_bytes"
fi

printf 'Alpine Studio product boundary checks passed\n'
