#!/bin/sh
set -eu

usage() {
    printf '%s\n' \
        'usage: scripts/check-worktrees.sh [--check] [--repo PATH] [--max-count N] [--require-single] [--plan-remove PATH] [--offline]' >&2
    exit 2
}

repo=.
max_count=3
require_single=false
plan_remove=
offline=false

while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo)
            [ "$#" -ge 2 ] || usage
            repo=$2
            shift 2
            ;;
        --max-count)
            [ "$#" -ge 2 ] || usage
            max_count=$2
            shift 2
            ;;
        --require-single)
            require_single=true
            shift
            ;;
        --check)
            shift
            ;;
        --plan-remove)
            [ "$#" -ge 2 ] || usage
            plan_remove=$2
            shift 2
            ;;
        --offline)
            offline=true
            shift
            ;;
        *) usage ;;
    esac
done

case "$max_count" in
    ''|*[!0-9]*) usage ;;
esac
[ "$max_count" -gt 0 ] || usage

repo_root=$(git -C "$repo" rev-parse --show-toplevel 2>/dev/null) || {
    printf 'worktree check error: %s is not a Git worktree\n' "$repo" >&2
    exit 1
}
repo_root=$(CDPATH= cd -- "$repo_root" && pwd -P)

temporary=$(mktemp -d "${TMPDIR:-/tmp}/alpine-worktrees.XXXXXX")
cleanup() { find "$temporary" -depth -delete; }
trap cleanup EXIT HUP INT TERM

raw=$temporary/worktrees.porcelain
manifest=$temporary/worktrees.tsv
active_branches=$temporary/active-pr-branches
git -C "$repo_root" worktree list --porcelain > "$raw"
awk '
    function emit() {
        if (path != "") {
            print path "\t" head "\t" branch
        }
    }
    /^worktree / {
        emit()
        path = substr($0, 10)
        head = ""
        branch = ""
        next
    }
    /^HEAD / { head = substr($0, 6); next }
    /^branch / { branch = substr($0, 8); next }
    /^detached$/ { branch = "(detached)"; next }
    END { emit() }
' "$raw" > "$manifest"

registered_count=$(wc -l < "$manifest" | tr -d ' ')
present_count=0
missing_count=0
active_known=false
: > "$active_branches"

if [ "${ALPINE_ACTIVE_PR_BRANCHES+x}" = x ]; then
    printf '%s\n' "$ALPINE_ACTIVE_PR_BRANCHES" | tr ' ' '\n' | sed '/^$/d; s#^refs/heads/##' | sort -u > "$active_branches"
    active_known=true
elif [ "$offline" = false ] && command -v gh >/dev/null 2>&1; then
    origin=$(git -C "$repo_root" remote get-url origin 2>/dev/null || true)
    slug=$(printf '%s\n' "$origin" | sed -n \
        -e 's#^git@github.com:\([^/][^/]*/[^/][^/]*\)\(.git\)\{0,1\}$#\1#p' \
        -e 's#^https\{0,1\}://github.com/\([^/][^/]*/[^/][^/]*\)\(.git\)\{0,1\}$#\1#p' \
        | sed 's/\.git$//' | head -n 1)
    if [ -n "$slug" ] && gh pr list --repo "$slug" --state open --limit 200 --json headRefName \
        --jq '.[].headRefName' > "$active_branches" 2>/dev/null; then
        sort -u "$active_branches" -o "$active_branches"
        active_known=true
    fi
fi

main_ref=HEAD
for candidate in refs/remotes/origin/main refs/heads/main; do
    if git -C "$repo_root" rev-parse --verify --quiet "$candidate^{commit}" >/dev/null; then
        main_ref=$candidate
        break
    fi
done

plan_path=$plan_remove
if [ -n "$plan_remove" ] && [ -d "$plan_remove" ]; then
    plan_path=$(CDPATH= cd -- "$plan_remove" && pwd -P)
fi
plan_seen=0
plan_allowed=0

printf 'path\thead\tbranch\tpresent\tdirty\tmain_relation\tactive_pr\tarchive\tdisposition\n'
tab=$(printf '\t')
while IFS="$tab" read -r path head branch_ref; do
    [ -n "$path" ] || continue
    branch=${branch_ref#refs/heads/}
    present=no
    dirty=unknown
    canonical_path=$path
    if [ -d "$path" ]; then
        present=yes
        present_count=$((present_count + 1))
        canonical_path=$(CDPATH= cd -- "$path" && pwd -P)
        if [ -n "$(git -C "$path" status --porcelain --untracked-files=normal 2>/dev/null || printf '?')" ]; then
            dirty=yes
        else
            dirty=no
        fi
    else
        missing_count=$((missing_count + 1))
    fi

    relation=ambiguous
    if [ -n "$head" ] && git -C "$repo_root" cat-file -e "$head^{commit}" 2>/dev/null; then
        if git -C "$repo_root" merge-base --is-ancestor "$head" "$main_ref" 2>/dev/null; then
            relation=merged
        else
            relation=unique
        fi
    fi

    active_pr=unknown
    if [ "$branch" = '(detached)' ] || [ -z "$branch" ]; then
        active_pr=not-applicable
    elif [ "$active_known" = true ]; then
        if grep -Fxq "$branch" "$active_branches"; then
            active_pr=yes
        else
            active_pr=no
        fi
    fi

    archive=no
    if [ -n "$head" ] && git -C "$repo_root" for-each-ref --format='%(objectname)' refs/archive/worktrees \
        | grep -Fxq "$head"; then
        archive=yes
    fi

    disposition=refuse-ambiguous
    if [ "$canonical_path" = "$repo_root" ]; then
        disposition=retain-authoritative
    elif [ "$present" = no ]; then
        disposition=refuse-missing-registration
    elif [ "$dirty" = yes ]; then
        disposition=refuse-dirty
    elif [ "$branch" = '(detached)' ] || [ -z "$branch" ]; then
        disposition=refuse-detached
    elif [ "$active_pr" = yes ]; then
        disposition=refuse-active-pr
    elif [ "$active_pr" = unknown ]; then
        disposition=refuse-ambiguous-pr
    elif [ "$relation" = merged ]; then
        disposition=removable-merged
    elif [ "$relation" = unique ] && [ "$archive" = yes ]; then
        disposition=removable-archived
    elif [ "$relation" = unique ]; then
        disposition=archive-required
    fi

    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$path" "$head" "${branch:-'(none)'}" "$present" "$dirty" "$relation" \
        "$active_pr" "$archive" "$disposition"

    if [ -n "$plan_remove" ] && { [ "$path" = "$plan_path" ] || [ "$canonical_path" = "$plan_path" ]; }; then
        plan_seen=$((plan_seen + 1))
        case "$disposition" in
            removable-merged|removable-archived) plan_allowed=1 ;;
        esac
    fi
done < "$manifest"

failures=0
if [ "$registered_count" -gt "$max_count" ]; then
    printf 'worktree check error: registered count %s exceeds limit %s\n' "$registered_count" "$max_count" >&2
    failures=$((failures + 1))
fi
if [ "$present_count" -gt "$max_count" ]; then
    printf 'worktree check error: present count %s exceeds limit %s\n' "$present_count" "$max_count" >&2
    failures=$((failures + 1))
fi
if [ "$missing_count" -ne 0 ]; then
    printf 'worktree check error: %s missing registration(s) require recorded disposition before prune\n' "$missing_count" >&2
    failures=$((failures + 1))
fi
if [ "$require_single" = true ] && { [ "$registered_count" -ne 1 ] || [ "$present_count" -ne 1 ]; }; then
    printf 'worktree check error: closure requires exactly one registered and present worktree\n' >&2
    failures=$((failures + 1))
fi
if [ -n "$plan_remove" ]; then
    if [ "$plan_seen" -ne 1 ]; then
        printf 'worktree check error: removal target must match exactly one registered worktree\n' >&2
        failures=$((failures + 1))
    elif [ "$plan_allowed" -ne 1 ]; then
        printf 'worktree check error: removal target is not safely removable\n' >&2
        failures=$((failures + 1))
    fi
fi

printf 'summary\tregistered=%s\tpresent=%s\tmissing=%s\tlimit=%s\tactive_pr_known=%s\n' \
    "$registered_count" "$present_count" "$missing_count" "$max_count" "$active_known"
[ "$failures" -eq 0 ] || exit 1
printf '%s\n' 'worktree inventory is bounded and classified'
