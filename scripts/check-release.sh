#!/bin/sh
set -eu

workspace_version=$(sed -nE 's/^version = "([0-9]+\.[0-9]+\.[0-9]+)"$/\1/p' Cargo.toml | head -n 1)

if [ -z "$workspace_version" ]; then
    printf 'release error: workspace.package.version must be plain SemVer\n' >&2
    exit 1
fi

if ! printf '%s\n' "$workspace_version" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'; then
    printf 'release error: invalid workspace SemVer %s\n' "$workspace_version" >&2
    exit 1
fi

for manifest in crates/*/Cargo.toml; do
    if ! grep -q '^version\.workspace = true$' "$manifest"; then
        printf 'release error: %s must inherit the workspace version\n' "$manifest" >&2
        exit 1
    fi

    if grep -qE 'path[[:space:]]*=' "$manifest" && grep -E 'path[[:space:]]*=' "$manifest" | grep -Ev "version[[:space:]]*=[[:space:]]*\"=$workspace_version\"" >/dev/null; then
        printf 'release error: %s path dependencies must pin the unified version =%s\n' "$manifest" "$workspace_version" >&2
        exit 1
    fi
done

if [ "$#" -gt 0 ]; then
    requested_tag=$1
    requested_version=${requested_tag#v}
    if [ "$requested_tag" != "v$workspace_version" ] || [ "$requested_version" != "$workspace_version" ]; then
        printf 'release error: requested tag %s does not match workspace version %s\n' "$requested_tag" "$workspace_version" >&2
        exit 1
    fi

    if git rev-parse --verify --quiet "refs/tags/$requested_tag" >/dev/null; then
        printf 'release error: immutable tag %s already exists\n' "$requested_tag" >&2
        exit 1
    fi
fi

printf 'release configuration checks passed for v%s\n' "$workspace_version"
