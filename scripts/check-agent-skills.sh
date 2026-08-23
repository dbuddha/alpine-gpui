#!/bin/sh
set -eu
failures=0
fail() { printf 'agent skill check error: %s\n' "$1" >&2; failures=$((failures + 1)); }
repo_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
skills_root=$repo_root/skills
if [ "$#" -gt 0 ]; then
    [ "$#" -eq 2 ] && [ "$1" = '--skills-root' ] || { printf 'usage: scripts/check-agent-skills.sh [--skills-root PATH]\n' >&2; exit 2; }
    skills_root=$(CDPATH= cd -- "$2" && pwd -P)
fi
skills='github-project-operator github-documentation-architect github-deep-researcher'
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
grep -Fq 'burn-up' "$skills_root/github-project-operator/SKILL.md" || fail 'project operator lacks burn-up guidance'
grep -Fq 'generated, revision-pinned retrieval mirror' "$skills_root/github-documentation-architect/SKILL.md" || fail 'documentation architect lacks Wiki authority rule'
grep -Fq 'Research lineage' "$skills_root/github-documentation-architect/SKILL.md" || fail 'documentation architect lacks research-lineage contract'
grep -Fq 'SUMMARY.md' "$skills_root/github-documentation-architect/SKILL.md" || fail 'documentation architect lacks navigation-only boundary'
grep -Fq 'Evidence Level' "$skills_root/github-project-operator/SKILL.md" || fail 'project operator lacks evidence-level handling'
grep -Fq 'Separate implementation tasks from qualification tasks' "$skills_root/github-project-operator/SKILL.md" || fail 'project operator lacks implementation-qualification split'
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
