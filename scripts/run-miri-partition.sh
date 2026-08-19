#!/bin/sh
set -eu

partition=${1:-}
toolchain=${MIRI_TOOLCHAIN:-nightly-2026-08-01}
manifest=assurance/miri-studio-partitions.tsv

run_packages() {
    cargo "+$toolchain" miri test "$@" --lib --locked -- --test-threads=1
}

run_studio() {
    filters=$(awk -F '\t' -v partition="$partition" '$2 == partition { print $1 }' "$manifest")
    if [ -z "$filters" ]; then
        printf 'unknown or empty Studio Miri partition: %s\n' "$partition" >&2
        exit 1
    fi
    all_filters=$(cut -f 1 "$manifest")
    for filter in $filters; do
        set -- --test-threads=1
        if [ "$filter" = 'tests::' ]; then
            for skip in $all_filters; do
                if [ "$skip" != 'tests::' ]; then
                    set -- "$@" --skip "$skip"
                fi
            done
        fi
        cargo "+$toolchain" miri test -p alpine-studio --lib --locked -- "$filter" "$@"
    done
}

case "$partition" in
    foundation)
        run_packages \
            -p alpine-core \
            -p alpine-metal \
            -p alpine-platform \
            -p alpine-platform-macos \
            -p alpine-renderer \
            -p alpine-runtime \
            -p alpine-scene \
            -p alpine-trace
        ;;
    studio-*)
        run_studio
        ;;
    text)
        run_packages -p alpine-text
        ;;
    text-layout)
        run_packages -p alpine-text-layout
        ;;
    *)
        printf 'unknown Miri partition: %s\n' "$partition" >&2
        exit 1
        ;;
esac
