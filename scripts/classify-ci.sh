#!/bin/sh
set -eu

base_sha=${ALPINE_BASE_SHA:-}
head_sha=${ALPINE_HEAD_SHA:-HEAD}
labels=${ALPINE_PR_LABELS:-}

if [ -z "$base_sha" ] || ! git cat-file -e "$base_sha^{commit}" 2>/dev/null; then
    base_sha=$(git rev-parse "$head_sha^")
fi

if [ -n "${ALPINE_CHANGED_FILES:-}" ]; then
    changed_files=$ALPINE_CHANGED_FILES
else
    changed_files=$(git diff --name-only "$base_sha...$head_sha")
fi

matches() {
    printf '%s\n' "$changed_files" | grep -Eq "$1"
}

has_label() {
    printf '%s\n' "$labels" | tr ',' '\n' | grep -Fxq "$1"
}

coverage=false
mutation=false
kani=false
miri=false
metal=false
tla=false

# Changes to the CI control plane must exercise every gate it can select.
# Classifying these paths selectively would allow the classifier or workflow
# itself to suppress the evidence required to prove its own correction.
if matches '^(\.github/workflows/(ci|nightly-assurance|release-dry-run)\.yml$|assurance/miri-studio-partitions\.tsv$|scripts/(classify-ci|test-classifier|setup-kani|test-setup-kani|test-studio-concurrency-stress|check-coverage|test-coverage|run-miri-partition|test-miri-partitions)\.sh$)'; then
    coverage=true
    mutation=true
    kani=true
    miri=true
    metal=true
    tla=true
fi

# Shipping application Rust receives the same coverage contract as crate Rust.
if matches '^(Cargo\.toml$|Cargo\.lock$|crates/.+\.rs$|crates/.+/Cargo\.toml$|apps/.+\.rs$|apps/.+/Cargo\.toml$|tools/[^/]+/(src/.+\.rs|Cargo\.toml)$|tools/alpine-trace/)'; then
    coverage=true
fi

if matches '^(crates/(alpine-core|alpine-scene|alpine-renderer|alpine-metal|alpine-platform|alpine-platform-macos|alpine-text|alpine-text-layout)/.+\.rs$|tools/alpine-trace/.+\.rs$)'; then
    mutation=true
    kani=true
fi

if matches '^(apps/alpine-studio/.+\.rs$|apps/alpine-studio/Cargo\.toml$)'; then
    mutation=true
fi

# Every Rust tool implementation receives changed-code mutation evidence.
# Tool-specific selectors below may require additional formal evidence, but a
# newly admitted tool must not bypass mutation merely because it is unnamed.
if matches '^tools/[^/]+/(src/.+\.rs|Cargo\.toml)$'; then
    mutation=true
fi

if matches '^(tools/alpine-assurance/.+\.rs$|assurance/qualification/)'; then
    coverage=true
    mutation=true
fi

if matches '^(formal/tla/|docs/aep/|assurance/evidence\.toml$|assurance/qualification/|tools/alpine-assurance/|tools/alpine-trace/)'; then
    tla=true
fi

if has_label review:unsafe || matches '^(crates/alpine-text-layout/|crates/.+/(unsafe|ffi|resource|lifetime))'; then
    miri=true
fi

if matches '^(apps/alpine-studio/(.+\.rs|Cargo\.toml)$|crates/(alpine-metal|alpine-platform-macos)/|tools/alpine-ax-client/(src/.+\.rs|Cargo\.toml)$|shaders/|.+\.metal$)'; then
    metal=true
fi

# Metal validation orchestration owns the same native evidence as the source it
# qualifies. A gate must not be able to change while classifying itself out.
if matches '^scripts/(check-metal|check-native-benchmark-result|test-native-benchmark-result)\.sh$'; then
    metal=true
fi

if [ -n "${GITHUB_OUTPUT:-}" ]; then
    {
        printf 'base_sha=%s\n' "$base_sha"
        printf 'head_sha=%s\n' "$head_sha"
        printf 'coverage=%s\n' "$coverage"
        printf 'mutation=%s\n' "$mutation"
        printf 'kani=%s\n' "$kani"
        printf 'miri=%s\n' "$miri"
        printf 'metal=%s\n' "$metal"
        printf 'tla=%s\n' "$tla"
    } >> "$GITHUB_OUTPUT"
else
    printf 'base_sha=%s\nhead_sha=%s\ncoverage=%s\nmutation=%s\nkani=%s\nmiri=%s\nmetal=%s\ntla=%s\n' \
        "$base_sha" "$head_sha" "$coverage" "$mutation" "$kani" "$miri" "$metal" "$tla"
fi
