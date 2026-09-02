#!/bin/sh
set -eu

targets='x86_64-unknown-linux-gnu x86_64-pc-windows-msvc'
installed=$(rustup target list --installed)

for target in $targets; do
    if ! printf '%s\n' "$installed" | rg -q "^${target}$"; then
        printf 'required portable compile target is missing: %s\n' "$target" >&2
        printf 'install it with the official command: rustup target add %s\n' "$target" >&2
        exit 1
    fi
done

for target in $targets; do
    cargo check \
        --workspace \
        --all-targets \
        --all-features \
        --locked \
        --target "$target"
done

printf 'portable Linux and Windows compile-contract checks passed\n'
