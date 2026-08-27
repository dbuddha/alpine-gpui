#!/bin/sh
set -eu

index_path=${1:-docs/research/index.md}
repository=${GITHUB_REPOSITORY:-dbuddha/alpine-gpui}
fail=0

error() {
    echo "research-index check failed: $1" >&2
    fail=1
}

require_file() {
    path=$1
    if [ ! -f "$path" ]; then
        error "missing required file: $path"
    fi
}

require_file "$index_path"
require_file docs/use-cases/alpine-studio-highfidelity.md
require_file docs/case-studies/zed-editor.md
require_file docs/case-studies/zed-gpui.md
require_file docs/case-studies/sublime-editor.md

if ! rg -q "Research evidence matrix" docs/use-cases/alpine-studio-highfidelity.md; then
    error "use-case matrix anchor missing in docs/use-cases/alpine-studio-highfidelity.md"
fi

if ! rg -q "### Requirement #31" "$index_path" || ! rg -q "### Requirement #40" "$index_path"; then
    error "requirement anchor coverage in docs/research/index.md is incomplete"
fi

if ! rg -q "workload_identity_hash" "$index_path"; then
    error "workload identity hash tags missing from docs/research/index.md"
fi

if ! rg -q "environment_hash" "$index_path"; then
    error "environment hash tags missing from docs/research/index.md"
fi

if [ ! -f scripts/check.sh ]; then
    error "scripts/check.sh not found"
fi

if ! rg -q "scripts/check-research-links.sh" scripts/check.sh; then
    error "check.sh does not invoke scripts/check-research-links.sh"
fi

if [ -x "$(command -v gh || true)" ] && gh auth status >/dev/null 2>&1; then
    requirements=$(gh issue list -R "$repository" --label kind:requirement --state open --json number --jq '.[].number' || true)
    if [ -n "$requirements" ]; then
        for req in $requirements; do
            if [ "$req" -lt 31 ] || [ "$req" -gt 40 ]; then
                continue
            fi
            if ! rg -q "^### Requirement #$req" "$index_path"; then
                error "docs/research/index.md missing anchor for requirement #$req"
            fi
        done
    fi

    issue_ids=$(rg -o 'issues/[0-9]+' "$index_path" | cut -d/ -f2 | sort -u || true)
    if [ -n "$issue_ids" ]; then
        for issue in $issue_ids; do
            case "$issue" in
                27|113|114|115|116)
                    ;;
                *)
                    continue
                    ;;
            esac

            labels=$(gh issue view "$issue" -R "$repository" --json labels --jq '[.labels[].name] | join("\n")' || true)
            if ! printf '%s\n' "$labels" | rg -q '^kind:research$'; then
                error "issue #$issue is linked in index but lacks kind:research"
            fi

            if [ "$issue" -gt 112 ]; then
                body=$(gh issue view "$issue" -R "$repository" --json body --jq '.body' || true)
                if ! printf '%s\n' "$body" | rg -q 'Capability anchor: #28'; then
                    error "issue #$issue is linked in index but lacks explicit Capability anchor #28"
                fi
            fi
        done
    fi
else
    echo "warning: gh auth unavailable; remote evidence checks skipped" >&2
fi

if [ "$fail" -ne 0 ]; then
    exit 1
fi

printf '%s\n' "research-index check passed"
