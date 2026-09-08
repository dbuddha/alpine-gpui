#!/bin/bash
set -euo pipefail
trap 'status=$?; printf "assurance collector: collection failed (exit %s)\n" "$status" >&2; exit "$status"' ERR

if [[ ! "${GITHUB_REPOSITORY:-}" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] \
    || [[ ! "${RUN_ID:-}" =~ ^[1-9][0-9]*$ ]]; then
    printf 'assurance collector: invalid repository or run identity\n' >&2
    exit 1
fi

temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM

# Buffer the complete selection before publication. An API/JSON failure must
# not publish a partial list that looks like complete failure reconciliation.
gh api "repos/$GITHUB_REPOSITORY/actions/runs/$RUN_ID/jobs" --paginate |
    jq -c '
        if (.jobs | type) != "array" then error("jobs must be an array")
        else .jobs[] end
        | select(.status == "completed")
        | select(.conclusion == "failure" or .conclusion == "cancelled"
            or .conclusion == "timed_out")
    ' > "$temporary/jobs"

while IFS= read -r job; do
    conclusion=$(jq -er '.conclusion' <<< "$job")
    reason=
    if [[ "$conclusion" == cancelled ]]; then
        check_url=$(jq -er '.check_run_url' <<< "$job")
        prefix="${GITHUB_API_URL:-https://api.github.com}/repos/$GITHUB_REPOSITORY/check-runs/"
        if [[ "$check_url" != "$prefix"* ]] \
            || [[ ! "${check_url#"$prefix"}" =~ ^[1-9][0-9]*$ ]]; then
            printf 'assurance collector: check URL is outside expected repository\n' >&2
            exit 1
        fi
        timeout_signal=$(
            gh api "$check_url/annotations" --paginate |
                jq -r --argjson job "$job" '
                    if type != "array" then error("annotations must be an array")
                    else .[] end
                    | select(.annotation_level == "failure" and .path == ".github")
                    | .message
                    | capture("^The job has exceeded the maximum execution time of (?:(?<hours>[0-9]+)h)?(?:(?<minutes>[0-9]+)m)?(?<seconds>[0-9]+)s$")?
                    | (((.hours // "0" | tonumber) * 3600)
                        + ((.minutes // "0" | tonumber) * 60)
                        + (.seconds | tonumber)) as $limit
                    | select($limit > 0)
                    | select((($job.completed_at | fromdateiso8601)
                        - ($job.started_at | fromdateiso8601)) >= $limit)
                    | "timeout"
                '
        )
        # Ordinary/manual and superseded cancellations have no qualifying
        # runner timeout signal. Do not manufacture a defect from cancellation.
        if [[ -z "$timeout_signal" ]]; then
            continue
        fi
        reason='job timed out: '
    elif [[ "$conclusion" == timed_out ]]; then
        reason='job timed out: '
    fi
    jq -r --arg reason "$reason" '
        [.name, ($reason + ([.steps[]?
            | select(.conclusion == "failure"
                or ($reason != "" and .conclusion == "cancelled"))
            | .name] | join(", ")))] | @tsv
    ' <<< "$job"
done < "$temporary/jobs" > "$temporary/selected"

cat "$temporary/selected"
