#!/bin/sh
set -eu

failures=0

fail() {
    printf 'policy error: %s\n' "$1" >&2
    failures=$((failures + 1))
}

workflow_files=$(find .github/workflows -type f \( -name '*.yml' -o -name '*.yaml' \) -print)

if [ -n "$workflow_files" ]; then
    action_refs=$(grep -hE '^[[:space:]]*uses:' $workflow_files || true)
    if [ -n "$action_refs" ] && printf '%s\n' "$action_refs" | grep -Ev '@[0-9a-f]{40}([[:space:]]|$)' >/dev/null; then
        fail 'every GitHub Action must be pinned to a full commit SHA'
        printf '%s\n' "$action_refs" | grep -Ev '@[0-9a-f]{40}([[:space:]]|$)' >&2 || true
    fi

    if grep -nE 'continue-on-error:[[:space:]]*true' $workflow_files >/dev/null; then
        fail 'CI gates may not use continue-on-error: true'
        grep -nE 'continue-on-error:[[:space:]]*true' $workflow_files >&2 || true
    fi
fi

manifest_files=$(find . -name Cargo.toml -not -path './target/*' -print)
if [ -n "$manifest_files" ] && grep -nE 'git[[:space:]]*=[[:space:]]*"https?://' $manifest_files >/dev/null; then
    fail 'shipping Cargo manifests may not contain Git dependencies'
    grep -nE 'git[[:space:]]*=[[:space:]]*"https?://' $manifest_files >&2 || true
fi

old_project='Ro''ck GPUI'
old_probe='ro''ck-metal-probe'
if git grep -n -I -e "$old_project" -e "$old_probe" -- . ':!scripts/check-policy.sh' >/dev/null 2>&1; then
    fail 'retired project names remain in tracked files'
    git grep -n -I -e "$old_project" -e "$old_probe" -- . ':!scripts/check-policy.sh' >&2 || true
fi

for fragment in changes/*.md; do
    [ -e "$fragment" ] || continue
    [ "$(basename "$fragment")" = 'README.md' ] && continue
    [ "$(basename "$fragment")" = 'AGENTS.md' ] && continue

    fragment_name=$(basename "$fragment")
    if ! printf '%s\n' "$fragment_name" | grep -Eq '^[A-Za-z0-9][A-Za-z0-9-]*\.(added|changed|deprecated|removed|fixed|performance|security)\.md$'; then
        fail "invalid change fragment name: $fragment_name"
    fi

    if [ ! -s "$fragment" ] || ! grep -q '[^[:space:]]' "$fragment"; then
        fail "empty change fragment: $fragment_name"
    fi
done

if [ -n "${ALPINE_BASE_SHA:-}" ] && [ -n "${ALPINE_HEAD_SHA:-}" ]; then
    changed_files=$(git diff --name-only "$ALPINE_BASE_SHA...$ALPINE_HEAD_SHA")
    shipping_changes=$(printf '%s\n' "$changed_files" | grep -E '^(Cargo\.toml$|Cargo\.lock$|crates/.+/Cargo\.toml$|crates/.+\.rs$|shaders/)' || true)
    fragment_changes=$(printf '%s\n' "$changed_files" | grep -E '^changes/[A-Za-z0-9][A-Za-z0-9-]*\.(added|changed|deprecated|removed|fixed|performance|security)\.md$' || true)

    if [ -n "$shipping_changes" ] && [ -z "$fragment_changes" ]; then
        fail 'shipping changes require a change fragment under changes/'
    fi
fi

if [ "$failures" -ne 0 ]; then
    exit 1
fi

printf 'repository policy checks passed\n'
