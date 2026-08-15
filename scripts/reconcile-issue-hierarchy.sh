#!/bin/sh
set -eu

repository=${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}
issue_number=${ALPINE_ISSUE_NUMBER:?ALPINE_ISSUE_NUMBER is required}
issue_action=${ALPINE_ISSUE_ACTION:?ALPINE_ISSUE_ACTION is required}
api_version=2026-03-10

issue_labels=$(gh issue view "$issue_number" --repo "$repository" \
    --json labels --jq '.labels[].name')
if printf '%s\n' "$issue_labels" | grep -Fxq kind:capability; then
    top_level=true
elif printf '%s\n' "$issue_labels" | grep -Eq '^kind:(requirement|task)$'; then
    top_level=false
else
    printf 'Issue #%s is outside the governed delivery hierarchy.\n' "$issue_number"
    exit 0
fi

if [ "$issue_action" = closed ]; then
    issue_subissues=$(gh api -H "X-GitHub-Api-Version: $api_version" \
        "repos/$repository/issues/$issue_number/sub_issues?per_page=100")
    issue_child_count=$(printf '%s\n' "$issue_subissues" | jq 'length')
    issue_children_closed=$(printf '%s\n' "$issue_subissues" | \
        jq 'length > 0 and all(.[]; .state == "closed")')
    if [ "$issue_child_count" -gt 0 ] && [ "$issue_children_closed" != true ]; then
        gh issue reopen "$issue_number" --repo "$repository" \
            --comment "Reopened automatically because one or more native sub-issues remain open."
        exit 0
    fi
fi

if [ "$top_level" = true ]; then
    exit 0
fi

parent=$(gh api -H "X-GitHub-Api-Version: $api_version" \
    "repos/$repository/issues/$issue_number/parent" --jq .number)
case "$parent" in
    '' | *[!0-9]*)
        printf 'Issue hierarchy error: invalid parent identity for #%s.\n' \
            "$issue_number" >&2
        exit 1
        ;;
esac

if [ "$issue_action" = reopened ]; then
    parent_state=$(gh issue view "$parent" --repo "$repository" --json state --jq .state)
    if [ "$parent_state" = CLOSED ]; then
        gh issue reopen "$parent" --repo "$repository" \
            --comment "Reopened automatically because child #$issue_number reopened."
    fi
    exit 0
fi

parent_labels=$(gh issue view "$parent" --repo "$repository" --json labels --jq '.labels[].name')
if ! printf '%s\n' "$parent_labels" | grep -Eq '^kind:(requirement|capability)$'; then
    printf 'Issue hierarchy error: parent #%s is not a requirement or capability.\n' "$parent" >&2
    exit 1
fi
if ! printf '%s\n' "$parent_labels" | grep -Fxq owner:approved; then
    printf 'Issue hierarchy error: parent #%s is not owner-approved.\n' "$parent" >&2
    exit 1
fi

if [ "${ALPINE_ENFORCE_EVIDENCE:-true}" = true ]; then
    if printf '%s\n' "$parent_labels" | grep -Fxq kind:requirement; then
        registry_field=requirement_issue
    else
        registry_field=capability_issue
    fi
    if ! grep -Eq "^$registry_field = $parent$" assurance/evidence.toml; then
        printf 'Issue hierarchy error: parent #%s has no registered assurance claims.\n' \
            "$parent" >&2
        exit 1
    fi
fi

parent_subissues=$(gh api -H "X-GitHub-Api-Version: $api_version" \
    "repos/$repository/issues/$parent/sub_issues?per_page=100")
parent_children_closed=$(printf '%s\n' "$parent_subissues" | \
    jq 'length > 0 and all(.[]; .state == "closed")')
if [ "$parent_children_closed" = true ]; then
    gh issue close "$parent" --repo "$repository" --reason completed \
        --comment "Closed automatically after all native sub-issues completed."
fi
