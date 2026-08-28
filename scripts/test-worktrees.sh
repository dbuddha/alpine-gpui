#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
checker=$repo_root/scripts/check-worktrees.sh
temporary=$(mktemp -d "${TMPDIR:-/tmp}/alpine-worktree-tests.XXXXXX")
cleanup() { find "$temporary" -depth -delete; }
trap cleanup EXIT HUP INT TERM

repository=$temporary/repository
mkdir -p "$repository"
git -C "$repository" init -q -b main
git -C "$repository" config user.name 'Alpine Worktree Tests'
git -C "$repository" config user.email 'worktrees@alpine.invalid'
printf '%s\n' base > "$repository/base.txt"
git -C "$repository" add base.txt
git -C "$repository" commit -q -m base
git -C "$repository" remote add origin "$repository"
git -C "$repository" update-ref refs/remotes/origin/main HEAD

ALPINE_ACTIVE_PR_BRANCHES= "$checker" --repo "$repository" --offline --require-single \
    > "$temporary/single.log"
grep -Fq 'registered=1' "$temporary/single.log"
grep -Fq 'retain-authoritative' "$temporary/single.log"

merged=$temporary/merged
git -C "$repository" branch merged
git -C "$repository" worktree add -q "$merged" merged
ALPINE_ACTIVE_PR_BRANCHES= "$checker" --repo "$repository" --offline --plan-remove "$merged" \
    > "$temporary/merged.log"
grep -Fq 'removable-merged' "$temporary/merged.log"

if ALPINE_ACTIVE_PR_BRANCHES=merged "$checker" --repo "$repository" --offline \
    --plan-remove "$merged" > "$temporary/active.log" 2>&1; then
    printf '%s\n' 'worktree test error: active PR candidate was removable' >&2
    exit 1
fi
grep -Fq 'refuse-active-pr' "$temporary/active.log"

printf '%s\n' dirty > "$merged/untracked.txt"
if ALPINE_ACTIVE_PR_BRANCHES= "$checker" --repo "$repository" --offline \
    --plan-remove "$merged" > "$temporary/dirty.log" 2>&1; then
    printf '%s\n' 'worktree test error: dirty candidate was removable' >&2
    exit 1
fi
grep -Fq 'refuse-dirty' "$temporary/dirty.log"
find "$merged" -maxdepth 1 -name untracked.txt -delete

if "$checker" --repo "$repository" --offline --plan-remove "$merged" \
    > "$temporary/unknown-pr.log" 2>&1; then
    printf '%s\n' 'worktree test error: unknown PR state was removable' >&2
    exit 1
fi
grep -Fq 'refuse-ambiguous-pr' "$temporary/unknown-pr.log"

unique=$temporary/unique
git -C "$repository" worktree add -q -b unique "$unique" main
printf '%s\n' unique > "$unique/unique.txt"
git -C "$unique" add unique.txt
git -C "$unique" commit -q -m unique
if ALPINE_ACTIVE_PR_BRANCHES= "$checker" --repo "$repository" --offline \
    --plan-remove "$unique" > "$temporary/unique.log" 2>&1; then
    printf '%s\n' 'worktree test error: unarchived unique candidate was removable' >&2
    exit 1
fi
grep -Fq 'archive-required' "$temporary/unique.log"
unique_head=$(git -C "$unique" rev-parse HEAD)
git -C "$repository" update-ref refs/archive/worktrees/test/unique "$unique_head"
ALPINE_ACTIVE_PR_BRANCHES= "$checker" --repo "$repository" --offline --plan-remove "$unique" \
    > "$temporary/archived.log"
grep -Fq 'removable-archived' "$temporary/archived.log"

detached=$temporary/detached
git -C "$repository" worktree add -q --detach "$detached" main
if ALPINE_ACTIVE_PR_BRANCHES= "$checker" --repo "$repository" --offline \
    --plan-remove "$detached" > "$temporary/detached.log" 2>&1; then
    printf '%s\n' 'worktree test error: detached candidate was removable' >&2
    exit 1
fi
grep -Fq 'refuse-detached' "$temporary/detached.log"

if ALPINE_ACTIVE_PR_BRANCHES= "$checker" --repo "$repository" --offline --max-count 3 \
    > "$temporary/limit.log" 2>&1; then
    printf '%s\n' 'worktree test error: excessive worktree count passed' >&2
    exit 1
fi
grep -Fq 'registered count 4 exceeds limit 3' "$temporary/limit.log"
if ALPINE_ACTIVE_PR_BRANCHES= "$checker" --repo "$repository" --offline --require-single \
    > "$temporary/closure.log" 2>&1; then
    printf '%s\n' 'worktree test error: multi-worktree closure passed' >&2
    exit 1
fi
grep -Fq 'closure requires exactly one registered and present worktree' "$temporary/closure.log"

git -C "$repository" worktree lock "$detached"
find "$detached" -depth -delete
if ALPINE_ACTIVE_PR_BRANCHES= "$checker" --repo "$repository" --offline --max-count 10 \
    > "$temporary/missing.log" 2>&1; then
    printf '%s\n' 'worktree test error: missing registration passed' >&2
    exit 1
fi
grep -Fq 'refuse-missing-registration' "$temporary/missing.log"
grep -Fq 'missing registration(s) require recorded disposition before prune' "$temporary/missing.log"

printf '%s\n' 'worktree inventory tests passed'
