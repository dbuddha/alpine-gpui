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

for retired_path in \
    PRODUCT.md \
    CHANGELOG.md \
    CONTRIBUTING.md \
    crates/AGENTS.md \
    crates/README.md \
    .github/AGENTS.md
do
    if [ -e "$retired_path" ]; then
        fail "unexpected standing artifact: $retired_path"
    fi
done

for retired_directory in changes docs; do
    if [ -d "$retired_directory" ] && find "$retired_directory" -type f -print -quit | grep -q .; then
        fail "unexpected files under retired directory: $retired_directory"
    fi
done

deleted_references='PRODUCT\.md|CHANGELOG\.md|changes/|docs/(MASTER_PLAN|ROADMAP|adr|research|DEPENDENCIES|ci|engineering)|crates/(AGENTS|README)\.md|provenance-ledger'
if git grep -n -I -E "$deleted_references" -- . ':!scripts/check-policy.sh' >/dev/null 2>&1; then
    fail 'tracked files reference retired repository documents'
    git grep -n -I -E "$deleted_references" -- . ':!scripts/check-policy.sh' >&2 || true
fi

agent_lines=$(wc -l < AGENTS.md | tr -d ' ')
if [ "$agent_lines" -lt 80 ] || [ "$agent_lines" -gt 120 ]; then
    fail "AGENTS.md must remain between 80 and 120 lines, found $agent_lines"
fi

if ! sed -n '1,4p' AGENTS.md | grep -q '^schema: alpine-agent-policy/v1$'; then
    fail 'AGENTS.md must declare schema alpine-agent-policy/v1 in frontmatter'
fi

issue_form_count=$(find .github/ISSUE_TEMPLATE -maxdepth 1 -type f -name '*.yml' ! -name config.yml | wc -l | tr -d ' ')
if [ "$issue_form_count" -ne 5 ]; then
    fail "exactly five structured issue forms are required, found $issue_form_count"
fi

for release_label in release:breaking release:feature release:fix release:none; do
    if ! grep -q -- "- $release_label" .github/release.yml; then
        fail "release-note configuration is missing $release_label"
    fi
done

if [ -n "${ALPINE_PR_BODY:-}" ] || [ -n "${ALPINE_PR_TITLE:-}" ]; then
    release_label_count=$(printf '%s\n' "${ALPINE_PR_LABELS:-}" | tr ',' '\n' | grep -Ec '^release:(breaking|feature|fix|none)$' || true)
    if [ "$release_label_count" -ne 1 ]; then
        fail "pull requests require exactly one release-impact label, found $release_label_count"
    fi

    if ! printf '%s\n' "${ALPINE_PR_TITLE:-}" | grep -Eq '^(build|chore|ci|docs|feat|fix|perf|refactor|revert|test)(\([a-z0-9_-]+\))?!?: .+'; then
        fail 'pull request title must use a Conventional Commit summary'
    fi

    for heading in \
        '## Closing issue' \
        '## Parent journey' \
        '## Decision or research' \
        '## Acceptance evidence' \
        '## Risk and scope' \
        '## Test plan' \
        '## Performance and memory' \
        '## Release impact' \
        '## Dependencies, provenance, and unsafe code' \
        '## Adversarial review'
    do
        if ! printf '%s\n' "$ALPINE_PR_BODY" | grep -Fqx "$heading"; then
            fail "pull request body is missing required heading: $heading"
        fi
    done
fi

if [ -n "${ALPINE_BASE_SHA:-}" ] && [ -n "${ALPINE_HEAD_SHA:-}" ]; then
    changed_files=$(git diff --name-only "$ALPINE_BASE_SHA...$ALPINE_HEAD_SHA")
    implementation_changes=$(printf '%s\n' "$changed_files" | grep -E '^(Cargo\.toml$|Cargo\.lock$|crates/.+/Cargo\.toml$|crates/.+\.rs$|shaders/)' || true)

    if [ -n "$implementation_changes" ]; then
        closing_issue=$(printf '%s\n' "${ALPINE_PR_BODY:-}" | sed -nE 's/.*([Cc]loses|[Cc]ontributes to)[[:space:]]+#([0-9]+).*/\2/p' | head -n 1)
        if [ -z "$closing_issue" ]; then
            fail 'implementation pull requests must close or contribute to a requirement or task issue'
        elif [ -n "${GH_REPOSITORY:-}" ] && command -v gh >/dev/null 2>&1; then
            issue_kind=$(gh issue view "$closing_issue" --repo "$GH_REPOSITORY" --json labels --jq '.labels[].name' 2>/dev/null | grep -E '^kind:(requirement|task)$' || true)
            if [ -z "$issue_kind" ]; then
                fail "linked issue #$closing_issue must have kind:requirement or kind:task"
            fi
        fi
    fi

    if printf '%s\n' "$changed_files" | grep -Fxq ARCHITECTURE.md; then
        decision_section=$(printf '%s\n' "${ALPINE_PR_BODY:-}" | awk '/^## Decision or research$/{capture=1; next} /^## /{if (capture) exit} capture')
        if ! printf '%s\n' "$decision_section" | grep -Eq '(#[0-9]+|https://github\.com/[^/]+/[^/]+/issues/[0-9]+)'; then
            fail 'architecture changes must link an accepted decision issue'
        fi
    fi
fi

if printf '%s\n' "${ALPINE_PR_LABELS:-}" | tr ',' '\n' | grep -Fxq review:provenance; then
    if [ ! -f provenance.toml ]; then
        fail 'review:provenance requires provenance.toml in the same pull request'
    fi
    if ! printf '%s\n' "${ALPINE_PR_BODY:-}" | grep -Eq '(#[0-9]+|https://github\.com/[^/]+/[^/]+/issues/[0-9]+)'; then
        fail 'source-level influence must link its research issue'
    fi
fi

if [ "$failures" -ne 0 ]; then
    exit 1
fi

printf 'repository policy checks passed\n'
