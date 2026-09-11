#!/bin/bash
set -euo pipefail
# Only trusted default-branch workflow_run events invoke this script. Never
# check out or execute the source of an untrusted pull request in this workflow.
[[ "${GITHUB_REPOSITORY:-}" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]
[[ "${RUN_ID:-}" =~ ^[1-9][0-9]*$ ]]
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
run=$(gh api "repos/$GITHUB_REPOSITORY/actions/runs/$RUN_ID")
head=$(jq -er '.head_sha | select(test("^[0-9a-f]{40}$"))' <<< "$run")
status=$(jq -er '.status' <<< "$run")
conclusion=$(jq -r '.conclusion' <<< "$run")
branch=$(jq -er '.head_branch' <<< "$run")
[[ "$status" == completed && "$branch" == main ]] || exit 0
case "$conclusion" in failure|timed_out) ;; *) exit 0 ;; esac
name=$(jq -er '.name' <<< "$run")
url=$(jq -er '.html_url' <<< "$run")
# A rerun supersedes its earlier attempts, including at the same source SHA.
attempt=$(jq -er '.run_attempt' <<< "$run")
if [[ -n "${RUN_ATTEMPT:-}" && "$attempt" != "$RUN_ATTEMPT" ]]; then exit 0; fi
still_current() {
    local current latest
    current=$(gh api "repos/$GITHUB_REPOSITORY/git/ref/heads/main" --jq '.object.sha') || return 2
    [[ "$current" =~ ^[0-9a-f]{40}$ ]] || return 2
    [[ "$current" == "$head" ]] || return 1
    latest=$(gh api "repos/$GITHUB_REPOSITORY/actions/runs/$RUN_ID") || return 2
    jq -e --arg head "$head" --argjson attempt "$attempt" '
        .head_sha == $head and .head_branch == "main" and
        .run_attempt == $attempt and .status == "completed" and
        (.conclusion == "failure" or .conclusion == "timed_out")
    ' <<< "$latest" >/dev/null || return 1
}
require_current() {
    local result=0
    still_current || result=$?
    case "$result" in 0) ;; 1) exit 0 ;; *) exit "$result" ;; esac
}
require_current
scripts/collect-assurance-failures.sh | scripts/filter-assurance-failures.sh > "$temporary/failures"
while IFS=$'\t' read -r job steps; do
    # Recheck immediately before each publication, since collection can take time.
    require_current
    key=$(printf '%s\n' "$name|$job|$steps" | shasum -a 256 | cut -d' ' -f1)
    marker="alpine-ci-failure-$key"
    existing=$(gh issue list --repo "$GITHUB_REPOSITORY" --state open --search "$marker in:body" --json number --jq '.[0].number // empty')
    printf 'Workflow: %s\nJob: %s\nFailing steps: %s\nCommit: `%s`\nRun: %s\n' "$name" "$job" "$steps" "$head" "$url" > "$temporary/details"
    require_current
    if [[ -n "$existing" ]]; then
        gh issue comment "$existing" --repo "$GITHUB_REPOSITORY" --body-file "$temporary/details"
    else
        {
            printf '<!-- %s -->\n\n## Problem\n\nA check failed or timed out on current main.\n\n' "$marker"
            cat "$temporary/details"
            printf '\n## Observable acceptance\n\nFix the cause and identify relevant regression coverage. The affected required check passes on main. CI success alone does not declare product acceptance.\n'
        } > "$temporary/body"
        gh issue create --repo "$GITHUB_REPOSITORY" --title "[Defect] $name: $job" --body-file "$temporary/body"
    fi
done < "$temporary/failures"
