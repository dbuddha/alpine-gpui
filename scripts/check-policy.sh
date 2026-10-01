#!/bin/sh
set -eu

# Path allowlists use byte order, independent of the hosted runner's locale.
export LC_ALL=C

failures=0

fail() {
    printf 'policy error: %s\n' "$1" >&2
    failures=$((failures + 1))
}

# Prune the build tree and agent worktrees (full repository copies) instead of
# filtering afterwards. Untracked source manifests stay in the inventory.
manifest_files=$(find . \( -path './target' -o -path './.claude' \) -prune -o -name Cargo.toml -print)
if [ -n "$manifest_files" ] && grep -nE 'git[[:space:]]*=[[:space:]]*"https?://' $manifest_files >/dev/null; then
    fail 'shipping Cargo manifests may not contain Git dependencies'
    grep -nE 'git[[:space:]]*=[[:space:]]*"https?://' $manifest_files >&2 || true
fi

unsafe_override_files=$(grep -lE '^unsafe_code[[:space:]]*=[[:space:]]*"allow"' $manifest_files 2>/dev/null | sort || true)
expected_unsafe_override_files='./crates/alpine-metal/Cargo.toml
./crates/alpine-platform-macos/Cargo.toml
./crates/alpine-text-layout/Cargo.toml
./tools/alpine-ax-client/Cargo.toml'
if [ "$unsafe_override_files" != "$expected_unsafe_override_files" ]; then
    fail 'only audited native Metal, macOS platform, text, and non-shipping AX crates may override unsafe-code denial'
    printf '%s\n' "$unsafe_override_files" >&2
fi

unsafe_source_files=$(find crates apps tools -type f -name '*.rs' -print0 \
    | xargs -0 grep -lE 'unsafe[[:space:]]+(extern|fn|impl|trait)|unsafe[[:space:]]*[{(]' 2>/dev/null \
    | sort || true)
expected_unsafe_source_files='crates/alpine-metal/src/native.rs
crates/alpine-platform-macos/src/menu.rs
crates/alpine-platform-macos/src/native.rs
crates/alpine-platform-macos/src/native_accessibility.rs
crates/alpine-platform-macos/src/native_text_input.rs
crates/alpine-platform-macos/src/signpost.rs
crates/alpine-text-layout/src/native.rs
tools/alpine-ax-client/src/native.rs'
if [ "$unsafe_source_files" != "$expected_unsafe_source_files" ]; then
    fail 'unsafe Rust constructs must remain isolated in audited native boundary files'
    printf '%s\n' "$unsafe_source_files" >&2
fi

ci_files=$(find .github -type f \( -name '*.yml' -o -name '*.yaml' \) -print)
if [ -n "$ci_files" ]; then
    # Every uses: reference, including flow-style steps, must be a GitHub-owned
    # Action pinned to a full commit SHA.
    unpinned_refs=$(grep -ohE 'uses:[[:space:]]*[^[:space:]},]+' $ci_files \
        | grep -vE '^uses:[[:space:]]*(actions|github)/[A-Za-z0-9._/-]+@[0-9a-f]{40}$' \
        || true)
    if [ -n "$unpinned_refs" ]; then
        fail 'workflows may use only GitHub-owned Actions pinned to a full commit SHA'
        printf '%s\n' "$unpinned_refs" >&2
    fi

    # A non-blocking step or job would let a failed check report success.
    non_blocking=$(grep -nE '(^|[[:space:]{,])continue-on-error:' $ci_files || true)
    if [ -n "$non_blocking" ]; then
        fail 'workflow steps and jobs must stay blocking (no continue-on-error)'
        printf '%s\n' "$non_blocking" >&2
    fi

    broad_permissions=$(grep -nE 'permissions:[[:space:]]*write-all' $ci_files || true)
    if [ -n "$broad_permissions" ]; then
        fail 'workflows may not grant write-all permissions'
        printf '%s\n' "$broad_permissions" >&2
    fi

    issue_writers=$(grep -nE \
        'issues:[[:space:]]*write|gh[[:space:]]+issue[[:space:]]+create|issues\.create' \
        $ci_files || true)
    if [ -n "$issue_writers" ]; then
        fail 'workflows may not grant issues: write or file issues'
        printf '%s\n' "$issue_writers" >&2
    fi

    # A workflow naming a missing script fails only in CI, because the local
    # gate keeps its own list. Catch the drift here instead.
    missing_scripts=$(grep -hoE 'scripts/[A-Za-z0-9._-]+\.(sh|py)' $ci_files \
        | sort -u \
        | while IFS= read -r referenced; do
            [ -f "$referenced" ] || printf '%s\n' "$referenced"
        done)
    if [ -n "$missing_scripts" ]; then
        fail 'every script a workflow runs must exist'
        printf '%s\n' "$missing_scripts" >&2
    fi
fi

if [ "$failures" -ne 0 ]; then
    exit 1
fi

printf 'repository policy checks passed\n'
