#!/bin/sh
set -eu

failures=0

fail() {
    printf 'policy error: %s\n' "$1" >&2
    failures=$((failures + 1))
}

if [ "${GITHUB_EVENT_NAME:-}" = "pull_request" ] && [ -n "${GITHUB_REPOSITORY:-}" ] && [ -f "${GITHUB_EVENT_PATH:-}" ] && command -v gh >/dev/null 2>&1; then
    pull_request_number=$(jq -r '.pull_request.number // empty' "$GITHUB_EVENT_PATH" 2>/dev/null || true)
    if [ -z "$pull_request_number" ]; then
        pull_request_number=$(printf '%s\n' "${GITHUB_REF:-}" | sed -n 's#refs/pull/\([0-9]\+\)/.*#\1#p' || true)
    fi

    if [ -n "$pull_request_number" ]; then
        refreshed_pr_title=$(gh pr view "$pull_request_number" --repo "$GITHUB_REPOSITORY" --json title --jq .title 2>/dev/null || true)
        refreshed_pr_body=$(gh pr view "$pull_request_number" --repo "$GITHUB_REPOSITORY" --json body --jq .body 2>/dev/null || true)
        refreshed_pr_labels=$(gh pr view "$pull_request_number" --repo "$GITHUB_REPOSITORY" --json labels --jq '[.labels[].name] | join(",")' 2>/dev/null || true)

        if [ -n "$refreshed_pr_title" ]; then
            ALPINE_PR_TITLE=$refreshed_pr_title
        fi
        if [ -n "$refreshed_pr_body" ]; then
            ALPINE_PR_BODY=$refreshed_pr_body
        fi
        if [ -n "$refreshed_pr_labels" ]; then
            ALPINE_PR_LABELS=$refreshed_pr_labels
        fi
    fi
fi

workflow_files=$(find .github/workflows -type f \( -name '*.yml' -o -name '*.yaml' \) -print)

if [ -n "$workflow_files" ]; then
    action_refs=$(grep -hE '^[[:space:]]*uses:' $workflow_files || true)
    if [ -n "$action_refs" ] && printf '%s\n' "$action_refs" | grep -Ev '@[0-9a-f]{40}([[:space:]]|$)' >/dev/null; then
        fail 'every GitHub Action must be pinned to a full commit SHA'
        printf '%s\n' "$action_refs" | grep -Ev '@[0-9a-f]{40}([[:space:]]|$)' >&2 || true
    fi

    if [ -n "$action_refs" ] && printf '%s\n' "$action_refs" | grep -Ev 'uses:[[:space:]]+(actions|github)/' >/dev/null; then
        fail 'only GitHub-owned Actions are permitted'
        printf '%s\n' "$action_refs" | grep -Ev 'uses:[[:space:]]+(actions|github)/' >&2 || true
    fi

    if grep -nE 'continue-on-error:[[:space:]]*true' $workflow_files >/dev/null; then
        fail 'CI gates may not use continue-on-error: true'
        grep -nE 'continue-on-error:[[:space:]]*true' $workflow_files >&2 || true
    fi

    weekly_mutation_workflow=.github/workflows/weekly-assurance.yml
    if [ -f "$weekly_mutation_workflow" ]; then
        output_parent_line=$(grep -nF 'mkdir -p target' "$weekly_mutation_workflow" \
            | head -n 1 | cut -d: -f1 || true)
        mutation_line=$(grep -nF 'cargo mutants --workspace' "$weekly_mutation_workflow" \
            | head -n 1 | cut -d: -f1 || true)
        if [ -z "$output_parent_line" ] || [ -z "$mutation_line" ] \
            || [ "$output_parent_line" -ge "$mutation_line" ]; then
            fail 'weekly mutation must prepare its target output parent before cargo-mutants starts'
        fi
    fi
fi

manifest_files=$(find . -name Cargo.toml -not -path './target/*' -print)
if [ -n "$manifest_files" ] && grep -nE 'git[[:space:]]*=[[:space:]]*"https?://' $manifest_files >/dev/null; then
    fail 'shipping Cargo manifests may not contain Git dependencies'
    grep -nE 'git[[:space:]]*=[[:space:]]*"https?://' $manifest_files >&2 || true
fi

unsafe_override_files=$(grep -lE '^unsafe_code[[:space:]]*=[[:space:]]*"allow"' $manifest_files 2>/dev/null | sort || true)
expected_unsafe_override_files='./crates/alpine-metal/Cargo.toml
./crates/alpine-platform-macos/Cargo.toml
./crates/alpine-text-layout/Cargo.toml'
if [ "$unsafe_override_files" != "$expected_unsafe_override_files" ]; then
    fail 'only audited native Metal and macOS platform crates may override unsafe-code denial'
    printf '%s\n' "$unsafe_override_files" >&2
fi

unsafe_source_files=$(find crates -type f -name '*.rs' -print0 \
    | xargs -0 grep -lE 'unsafe[[:space:]]+(extern|fn|impl|trait)|unsafe[[:space:]]*\{' 2>/dev/null \
    | sort || true)
expected_unsafe_source_files='crates/alpine-metal/src/native.rs
crates/alpine-platform-macos/src/native.rs
crates/alpine-text-layout/src/native.rs'
if [ "$unsafe_source_files" != "$expected_unsafe_source_files" ]; then
    fail 'unsafe Rust constructs must remain isolated in audited native boundary files'
    printf '%s\n' "$unsafe_source_files" >&2
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

for retired_directory in changes; do
    if [ -d "$retired_directory" ] && find "$retired_directory" -type f -print -quit | grep -q .; then
        fail "unexpected files under retired directory: $retired_directory"
    fi
done

deleted_references='PRODUCT\.md|CHANGELOG\.md|changes/|docs/(MASTER_PLAN|ROADMAP|adr|DEPENDENCIES|ci|engineering)|crates/(AGENTS|README)\.md|provenance-ledger'
if [ "${ALPINE_POLICY_REFERENCE_INPUT+x}" = x ]; then
    retired_reference_matches=$(printf '%s\n' "$ALPINE_POLICY_REFERENCE_INPUT" \
        | grep -nE "$deleted_references" || true)
else
    retired_reference_matches=$(git grep -n -I -E "$deleted_references" -- . ':!scripts/check-policy.sh' || true)
fi
if [ -n "$retired_reference_matches" ]; then
    fail 'tracked files reference retired repository documents'
    printf '%s\n' "$retired_reference_matches" >&2
fi

research_files=$(find docs/research -type f -print 2>/dev/null | sort || true)
expected_research_files='docs/research/alpine-studio-adversarial-review.md
docs/research/index.md'
if [ "$research_files" != "$expected_research_files" ]; then
    fail 'docs/research may contain only the accepted catalog and adversarial review'
    printf '%s\n' "$research_files" >&2
fi

agent_lines=$(wc -l < AGENTS.md | tr -d ' ')
if [ "$agent_lines" -lt 80 ] || [ "$agent_lines" -gt 120 ]; then
    fail "AGENTS.md must remain between 80 and 120 lines, found $agent_lines"
fi

if ! sed -n '1,4p' AGENTS.md | grep -q '^schema: alpine-agent-policy/v1$'; then
    fail 'AGENTS.md must declare schema alpine-agent-policy/v1 in frontmatter'
fi

for required_path in \
    book.toml \
    docs/SUMMARY.md \
    docs/vision.md \
    docs/concepts/traceability.md \
    docs/quality/assurance.md \
    assurance/evidence.toml \
    assurance/alpine-studio-dependencies.txt \
    scripts/check-product-boundary.sh \
    scripts/test-product-boundary.sh \
    scripts/test-assurance.sh \
    scripts/test-metal-library.sh \
    scripts/verify-metal-library.sh \
    scripts/check-workload-hashes.sh \
    scripts/check-research-retention.sh \
    scripts/test-research-retention.sh \
    scripts/check-tla.sh \
    docs/research/index.md \
    docs/research/alpine-studio-adversarial-review.md \
    .cargo/mutants.toml
do
    if [ ! -f "$required_path" ]; then
        fail "required assurance artifact is missing: $required_path"
    fi
done

issue_form_count=$(find .github/ISSUE_TEMPLATE -maxdepth 1 -type f -name '*.yml' ! -name config.yml | wc -l | tr -d ' ')
if [ "$issue_form_count" -ne 5 ]; then
    fail "exactly five structured issue forms are required, found $issue_form_count"
fi

if [ ! -f .github/ISSUE_TEMPLATE/capability.yml ] || [ -f .github/ISSUE_TEMPLATE/user-journey.yml ]; then
    fail 'the top-level work item must be the capability issue form'
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
        '## Parent capability' \
        '## Claims and evidence' \
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
    if [ -n "${ALPINE_CHANGED_FILES:-}" ]; then
        changed_files=$ALPINE_CHANGED_FILES
    else
        changed_files=$(git diff --name-only "$ALPINE_BASE_SHA...$ALPINE_HEAD_SHA")
    fi
    implementation_changes=$(printf '%s\n' "$changed_files" | grep -E '^(Cargo\.toml$|Cargo\.lock$|crates/.+/Cargo\.toml$|crates/.+\.rs$|tools/alpine-trace/|shaders/)' || true)

    if [ -n "$implementation_changes" ] && { [ -n "${ALPINE_PR_BODY:-}" ] || [ -n "${ALPINE_PR_TITLE:-}" ]; }; then
        closing_issue=$(printf '%s\n' "${ALPINE_PR_BODY:-}" | sed -nE 's/.*([Cc]loses|[Cc]ontributes to)[[:space:]]+#([0-9]+).*/\2/p' | head -n 1)
        if [ -z "$closing_issue" ]; then
            fail 'implementation pull requests must close or contribute to a requirement or task issue'
        elif [ -n "${GH_REPOSITORY:-}" ] && command -v gh >/dev/null 2>&1; then
            issue_labels=$(gh issue view "$closing_issue" --repo "$GH_REPOSITORY" --json labels --jq '.labels[].name' 2>/dev/null || true)
            issue_state=$(gh issue view "$closing_issue" --repo "$GH_REPOSITORY" --json state --jq .state 2>/dev/null || true)
            issue_kind=$(printf '%s\n' "$issue_labels" | grep -E '^kind:(requirement|task)$' || true)
            if [ -z "$issue_kind" ]; then
                fail "linked issue #$closing_issue must have kind:requirement or kind:task"
            elif [ "$issue_state" != OPEN ]; then
                fail "linked issue #$closing_issue must be open"
            else
                issue_body=$(gh issue view "$closing_issue" --repo "$GH_REPOSITORY" --json body --jq .body 2>/dev/null || true)
                if printf '%s\n' "$issue_kind" | grep -Fxq kind:task; then
                    requirement_issue=$(printf '%s\n' "$issue_body" | awk '/^### Parent capability or requirement$/{capture=1; next} /^### /{if (capture) exit} capture' | grep -Eo '#[0-9]+' | tr -d '#' | head -n 1 || true)
                else
                    requirement_issue=$closing_issue
                fi

                if [ -z "${requirement_issue:-}" ]; then
                    fail "linked task #$closing_issue must name its parent requirement"
                else
                    if printf '%s\n' "$issue_kind" | grep -Fxq kind:task; then
                        native_requirement=$(gh api "repos/$GH_REPOSITORY/issues/$closing_issue/parent" --jq .number 2>/dev/null || true)
                        if [ "$native_requirement" != "$requirement_issue" ]; then
                            fail "task #$closing_issue must be a native sub-issue of requirement #$requirement_issue"
                        fi
                    fi

                    requirement_labels=$(gh issue view "$requirement_issue" --repo "$GH_REPOSITORY" --json labels --jq '.labels[].name' 2>/dev/null || true)
                    requirement_state=$(gh issue view "$requirement_issue" --repo "$GH_REPOSITORY" --json state --jq .state 2>/dev/null || true)
                    if ! printf '%s\n' "$requirement_labels" | grep -Fxq kind:requirement; then
                        fail "parent issue #$requirement_issue must have kind:requirement"
                    fi
                    if [ "$requirement_state" != OPEN ]; then
                        fail "requirement #$requirement_issue must be open"
                    fi
                    if ! printf '%s\n' "$requirement_labels" | grep -Fxq owner:approved; then
                        fail "requirement #$requirement_issue requires owner:approved"
                    fi

                    requirement_body=$(gh issue view "$requirement_issue" --repo "$GH_REPOSITORY" --json body --jq .body 2>/dev/null || true)
                    capability_issue=$(printf '%s\n' "$requirement_body" | awk '/^### Parent capability or requirement$/{capture=1; next} /^### /{if (capture) exit} capture' | grep -Eo '#[0-9]+' | tr -d '#' | head -n 1 || true)
                    if [ -z "$capability_issue" ]; then
                        fail "requirement #$requirement_issue must name its parent capability"
                    else
                        native_capability=$(gh api "repos/$GH_REPOSITORY/issues/$requirement_issue/parent" --jq .number 2>/dev/null || true)
                        if [ "$native_capability" != "$capability_issue" ]; then
                            fail "requirement #$requirement_issue must be a native sub-issue of capability #$capability_issue"
                        fi

                        capability_labels=$(gh issue view "$capability_issue" --repo "$GH_REPOSITORY" --json labels --jq '.labels[].name' 2>/dev/null || true)
                        capability_state=$(gh issue view "$capability_issue" --repo "$GH_REPOSITORY" --json state --jq .state 2>/dev/null || true)
                        if ! printf '%s\n' "$capability_labels" | grep -Fxq kind:capability; then
                            fail "parent issue #$capability_issue must have kind:capability"
                        fi
                        if [ "$capability_state" != OPEN ]; then
                            fail "capability #$capability_issue must be open"
                        fi
                        if ! printf '%s\n' "$capability_labels" | grep -Fxq owner:approved; then
                            fail "capability #$capability_issue requires owner:approved"
                        fi

                        pr_capability_section=$(printf '%s\n' "${ALPINE_PR_BODY:-}" | awk '/^## Parent capability$/{capture=1; next} /^## /{if (capture) exit} capture')
                        pr_capability=$(printf '%s\n' "$pr_capability_section" | grep -Eo '(#[0-9]+|https://github\.com/[^/]+/[^/]+/issues/[0-9]+)' | sed -E 's#^.*/issues/##; s/^#//' | head -n 1 || true)
                        if [ "$pr_capability" != "$capability_issue" ]; then
                            fail "pull request must link parent capability #$capability_issue"
                        fi
                    fi
                fi
            fi
        fi
    fi

    if { [ -n "${ALPINE_PR_BODY:-}" ] || [ -n "${ALPINE_PR_TITLE:-}" ]; } && printf '%s\n' "$changed_files" | grep -Fxq ARCHITECTURE.md; then
        decision_section=$(printf '%s\n' "${ALPINE_PR_BODY:-}" | awk '/^## Decision or research$/{capture=1; next} /^## /{if (capture) exit} capture')
        decision_issues=$(printf '%s\n' "$decision_section" | grep -Eo '(#[0-9]+|https://github\.com/[^/]+/[^/]+/issues/[0-9]+)' | sed -E 's#^.*/issues/##; s/^#//' | awk '!seen[$0]++' || true)
        if [ -z "$decision_issues" ]; then
            fail 'architecture changes must link an accepted decision issue'
        elif [ -n "${GH_REPOSITORY:-}" ] && command -v gh >/dev/null 2>&1; then
            accepted_decision=false
            for decision_issue in $decision_issues; do
                decision_metadata=$(gh issue view "$decision_issue" --repo "$GH_REPOSITORY" --json labels,state,stateReason --jq '[.state, .stateReason, (.labels[].name)] | @tsv' 2>/dev/null || true)
                if printf '%s\n' "$decision_metadata" | grep -Fq CLOSED && printf '%s\n' "$decision_metadata" | grep -Fq COMPLETED && printf '%s\n' "$decision_metadata" | grep -Fq kind:decision; then
                    accepted_decision=true
                fi
            done
            if [ "$accepted_decision" != true ]; then
                fail 'architecture changes require a closed kind:decision issue'
            fi
        fi
    fi
fi

if printf '%s\n' "${ALPINE_PR_LABELS:-}" | tr ',' '\n' | grep -Fxq review:provenance; then
    if [ ! -f provenance.toml ]; then
        fail 'review:provenance requires provenance.toml in the same pull request'
    fi
    research_section=$(printf '%s\n' "${ALPINE_PR_BODY:-}" | awk '/^## Decision or research$/{capture=1; next} /^## /{if (capture) exit} capture')
    research_issues=$(printf '%s\n' "$research_section" | grep -Eo '(#[0-9]+|https://github\.com/[^/]+/[^/]+/issues/[0-9]+)' | sed -E 's#^.*/issues/##; s/^#//' | awk '!seen[$0]++' || true)
    if [ -z "$research_issues" ]; then
        fail 'source-level influence must link its research issue'
    elif [ -n "${GH_REPOSITORY:-}" ] && command -v gh >/dev/null 2>&1; then
        accepted_research=false
        for research_issue in $research_issues; do
            research_metadata=$(gh issue view "$research_issue" --repo "$GH_REPOSITORY" --json labels,state,stateReason --jq '[.state, .stateReason, (.labels[].name)] | @tsv' 2>/dev/null || true)
            if printf '%s\n' "$research_metadata" | grep -Fq CLOSED && printf '%s\n' "$research_metadata" | grep -Fq COMPLETED && printf '%s\n' "$research_metadata" | grep -Fq kind:research; then
                accepted_research=true
            fi
        done
        if [ "$accepted_research" != true ]; then
            fail 'source-level influence requires a closed kind:research issue'
        fi
    fi
fi

if [ "$failures" -ne 0 ]; then
    exit 1
fi

printf 'repository policy checks passed\n'
