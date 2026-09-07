#!/bin/sh
set -eu
# This is a deterministic policy check over caller-supplied evidence fields,
# not a GitHub fetch, merge authorization, or atomic source/base lock.
if [ "${1-}" = '--merge-readiness' ]; then
    shift
    [ "$#" -eq 12 ] || {
        printf '%s\n' 'usage: scripts/check-agent-skills.sh --merge-readiness MODE DEFAULT_BRANCH BASE_BRANCH PROTECTION EXPECTED_HEAD RUN_HEAD EXPECTED_BASE TESTED_BASE AGGREGATE REQUIRED_CHECKS SELECTED_COUNT MERGEABILITY' >&2
        exit 2
    }
    merge_fail() { printf 'merge readiness policy error: %s\n' "$1" >&2; exit 1; }
    mode=$1; default_branch=$2; base_branch=$3; protection=$4
    expected_head=$5; run_head=$6; expected_base=$7; tested_base=$8
    aggregate=$9; required_checks=${10}; selected_count=${11}; mergeability=${12}
    case "$mode" in manual|auto) ;; *) merge_fail 'unknown merge mode' ;; esac
    [ -n "$default_branch" ] && [ -n "$base_branch" ] || merge_fail 'missing branch identity'
    case "$protection" in protected|unprotected) ;; *) merge_fail 'unknown base protection' ;; esac
    for revision in "$expected_head" "$run_head" "$expected_base" "$tested_base"; do
        printf '%s\n' "$revision" | LC_ALL=C grep -Eq '^[0-9a-f]{40}$' || merge_fail 'invalid source or base revision'
        [ "$revision" != '0000000000000000000000000000000000000000' ] || merge_fail 'missing source or base revision'
    done
    [ "$expected_head" = "$run_head" ] || merge_fail 'hosted run is not the expected source head'
    [ "$expected_base" = "$tested_base" ] || merge_fail 'tested base does not match the current base'
    [ "$aggregate" = success ] || merge_fail 'aggregate is not terminal-successful'
    [ "$required_checks" = success ] || merge_fail 'required checks are not all terminal-successful'
    case "$selected_count" in ''|0*|*[!0-9]*) merge_fail 'selected check count must be positive' ;; esac
    [ "${#selected_count}" -le 4 ] || merge_fail 'selected check count exceeds the policy bound'
    [ "$mergeability" = mergeable ] || merge_fail 'mergeability is not known and clean'
    if [ "$mode" = auto ]; then
        [ "$base_branch" = "$default_branch" ] && [ "$protection" = protected ] ||
            merge_fail 'auto-merge requires the protected default branch'
    fi
    printf '%s\n' 'merge snapshot satisfies policy; live GitHub verification and source-safe merge are still required'
    exit 0
fi
failures=0
fail() { printf 'agent skill check error: %s\n' "$1" >&2; failures=$((failures + 1)); }
repo_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
skills_root=$repo_root/skills
if [ "$#" -gt 0 ]; then
    [ "$#" -eq 2 ] && [ "$1" = '--skills-root' ] || { printf 'usage: scripts/check-agent-skills.sh [--skills-root PATH]\n' >&2; exit 2; }
    skills_root=$(CDPATH= cd -- "$2" && pwd -P)
fi
. "$repo_root/scripts/lib/agent-skills.sh"
skills=$(agent_skill_names "$repo_root" "$skills_root")
for skill in $skills; do
    directory=$skills_root/$skill; document=$directory/SKILL.md; agent=$directory/agents/openai.yaml
    [ -f "$document" ] || { fail "missing $skill/SKILL.md"; continue; }
    [ -f "$agent" ] || { fail "missing $skill/agents/openai.yaml"; continue; }
    [ "$(sed -n '1p' "$document")" = '---' ] || fail "$skill frontmatter does not open"
    [ "$(sed -n '2p' "$document")" = "name: $skill" ] || fail "$skill has the wrong frontmatter name"
    sed -n '3p' "$document" | grep -Eq '^description: .{80,}$' || fail "$skill needs a trigger-rich frontmatter description"
    [ "$(sed -n '4p' "$document")" = '---' ] || fail "$skill frontmatter does not close"
    grep -Fq 'display_name:' "$agent" || fail "$skill agent metadata lacks display_name"
    grep -Fq 'short_description:' "$agent" || fail "$skill agent metadata lacks short_description"
    grep -Fq "\$$skill" "$agent" || fail "$skill default prompt does not invoke the skill"
    grep -R -n -E 'TODO|TBD|FIXME' "$directory" >/dev/null 2>&1 && fail "$skill contains unfinished placeholders"
    links=$(sed -n 's/.*](\([^)]*\)).*/\1/p' "$document" || true)
    for link in $links; do case "$link" in http://*|https://*|\#*) ;; *) [ -f "$directory/$link" ] || fail "$skill references missing resource $link" ;; esac; done
done
grep -Fq 'read-only snapshot' "$skills_root/github-project-operator/SKILL.md" || fail 'project operator lacks read-before-write behavior'
grep -Fq 'PR metadata preflight' "$skills_root/github-project-operator/SKILL.md" || fail 'project operator lacks PR metadata preflight'
grep -Fq 'protected default branch' "$skills_root/github-project-operator/SKILL.md" || fail 'project operator lacks the auto-merge protection boundary'
grep -Fq -- '--merge-readiness' "$skills_root/github-project-operator/SKILL.md" || fail 'project operator lacks the merge-readiness policy command'
grep -Fq 'burn-up' "$skills_root/github-project-operator/SKILL.md" || fail 'project operator lacks burn-up guidance'
grep -Fq 'native blocked-by relationships' "$skills_root/github-project-operator/SKILL.md" || fail 'project operator lacks native dependency authority'
grep -Fq '`Delivery Gate`' "$skills_root/github-project-operator/SKILL.md" || fail 'project operator lacks the live delivery field schema'
grep -Fq 'generated, revision-pinned retrieval mirror' "$skills_root/github-documentation-architect/SKILL.md" || fail 'documentation architect lacks Wiki authority rule'
grep -Fq 'PR metadata preflight' "$skills_root/github-documentation-architect/SKILL.md" || fail 'documentation architect lacks PR metadata preflight'
grep -Fq 'Research lineage' "$skills_root/github-documentation-architect/SKILL.md" || fail 'documentation architect lacks research-lineage contract'
grep -Fq 'SUMMARY.md' "$skills_root/github-documentation-architect/SKILL.md" || fail 'documentation architect lacks navigation-only boundary'
grep -Fq 'live remote drift audit' "$skills_root/github-documentation-architect/SKILL.md" || fail 'documentation architect lacks live Wiki drift behavior'
grep -Fq 'native issue hierarchy and dependencies' "$skills_root/github-documentation-architect/SKILL.md" || fail 'documentation architect lacks reconciliation ordering'
grep -Fq 'scripts/wiki.sh audit-remote' "$skills_root/github-documentation-architect/references/wiki-and-releases.md" || fail 'documentation architect lacks the live Wiki audit command'
grep -Fq 'Evidence Level' "$skills_root/github-project-operator/SKILL.md" || fail 'project operator lacks evidence-level handling'
grep -Fq 'Separate implementation tasks from qualification tasks' "$skills_root/github-project-operator/SKILL.md" || fail 'project operator lacks implementation-qualification split'
grep -Fq 'read-only worktree inventory' "$skills_root/github-project-operator/SKILL.md" || fail 'project operator lacks worktree preflight behavior'
grep -Fq 'scripts/check-worktrees.sh --plan-remove' "$skills_root/github-project-operator/references/worktree-hygiene.md" || fail 'project operator lacks the worktree removal dry run'
grep -Fq 'E4 Qualified' "$skills_root/github-deep-researcher/SKILL.md" || fail 'deep researcher lacks qualification evidence level'
grep -Fq 'benchmark contamination' "$skills_root/github-deep-researcher/references/evidence-standard.md" || fail 'deep researcher lacks contamination analysis'
if [ "$skills_root" = "$repo_root/skills" ]; then
    for script in scripts/install-agent-skills.sh scripts/check-agent-skills.sh scripts/test-agent-skills.sh; do [ -x "$repo_root/$script" ] || fail "$script is not executable"; done
    grep -Fq 'github-project-operator' "$repo_root/AGENTS.md" || fail 'AGENTS.md lacks project skill trigger'
    grep -Fq 'github-documentation-architect' "$repo_root/AGENTS.md" || fail 'AGENTS.md lacks documentation skill trigger'
    grep -Fq 'github-deep-researcher' "$repo_root/AGENTS.md" || fail 'AGENTS.md lacks research skill trigger'
    grep -Fq 'operations/github-agent-skills.md' "$repo_root/docs/SUMMARY.md" || fail 'mdBook navigation lacks skill operator guide'
fi
[ "$failures" -eq 0 ] || exit 1
printf 'repository agent skills are valid\n'
