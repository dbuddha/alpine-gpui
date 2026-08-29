#!/bin/sh
set -eu
repo_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
installer=$repo_root/scripts/install-agent-skills.sh
checker=$repo_root/scripts/check-agent-skills.sh
temporary=$(mktemp -d "${TMPDIR:-/tmp}/alpine-agent-skills.XXXXXX")
cleanup() { find "$temporary" -depth -delete; }
trap cleanup EXIT HUP INT TERM
CODEX_HOME=$temporary/codex "$installer" --install >/dev/null
CODEX_HOME=$temporary/codex "$installer" --check >/dev/null
CODEX_HOME=$temporary/codex "$installer" --install >/dev/null
for skill in github-project-operator github-documentation-architect github-deep-researcher; do [ -L "$temporary/codex/skills/$skill" ]; done
CODEX_HOME=$temporary/codex "$installer" --remove-links >/dev/null
for skill in github-project-operator github-documentation-architect github-deep-researcher; do [ ! -e "$temporary/codex/skills/$skill" ]; done
mkdir -p "$temporary/codex/skills/github-project-operator"
if CODEX_HOME=$temporary/codex "$installer" --install >"$temporary/foreign-install.log" 2>&1; then printf 'agent skill test error: foreign destination unexpectedly replaced\n' >&2; exit 1; fi
[ ! -e "$temporary/codex/skills/github-documentation-architect" ]
rmdir "$temporary/codex/skills/github-project-operator"
ln -s "$temporary/foreign-target" "$temporary/codex/skills/github-deep-researcher"
if CODEX_HOME=$temporary/codex "$installer" --remove-links >"$temporary/foreign-remove.log" 2>&1; then printf 'agent skill test error: foreign link unexpectedly removed\n' >&2; exit 1; fi
[ -L "$temporary/codex/skills/github-deep-researcher" ]
unlink "$temporary/codex/skills/github-deep-researcher"
mkdir -p "$temporary/bad-skills"
cp -R "$repo_root/skills/." "$temporary/bad-skills"
awk 'NR == 2 { print "name: wrong-name"; next } { print }' "$temporary/bad-skills/github-project-operator/SKILL.md" > "$temporary/malformed-skill.md"
mv "$temporary/malformed-skill.md" "$temporary/bad-skills/github-project-operator/SKILL.md"
if "$checker" --skills-root "$temporary/bad-skills" >"$temporary/malformed.log" 2>&1; then printf 'agent skill test error: malformed skill unexpectedly passed\n' >&2; exit 1; fi
grep -Fq 'wrong frontmatter name' "$temporary/malformed.log"
rm -rf "$temporary/bad-skills"
cp -R "$repo_root/skills/." "$temporary/bad-skills"
sed 's/Research lineage/Research origin/' "$temporary/bad-skills/github-documentation-architect/SKILL.md" > "$temporary/missing-lineage.md"
mv "$temporary/missing-lineage.md" "$temporary/bad-skills/github-documentation-architect/SKILL.md"
if "$checker" --skills-root "$temporary/bad-skills" >"$temporary/missing-lineage.log" 2>&1; then printf 'agent skill test error: missing lineage contract unexpectedly passed\n' >&2; exit 1; fi
grep -Fq 'lacks research-lineage contract' "$temporary/missing-lineage.log"
rm -rf "$temporary/bad-skills"
cp -R "$repo_root/skills/." "$temporary/bad-skills"
sed 's/live remote drift audit/live remote freshness review/' "$temporary/bad-skills/github-documentation-architect/SKILL.md" > "$temporary/missing-wiki-audit.md"
mv "$temporary/missing-wiki-audit.md" "$temporary/bad-skills/github-documentation-architect/SKILL.md"
if "$checker" --skills-root "$temporary/bad-skills" >"$temporary/missing-wiki-audit.log" 2>&1; then printf 'agent skill test error: missing live Wiki drift behavior unexpectedly passed\n' >&2; exit 1; fi
grep -Fq 'lacks live Wiki drift behavior' "$temporary/missing-wiki-audit.log"
find "$temporary/bad-skills" -depth -delete
cp -R "$repo_root/skills/." "$temporary/bad-skills"
sed 's/read-only worktree inventory/read-only checkout inventory/' "$temporary/bad-skills/github-project-operator/SKILL.md" > "$temporary/missing-worktree-preflight.md"
mv "$temporary/missing-worktree-preflight.md" "$temporary/bad-skills/github-project-operator/SKILL.md"
if "$checker" --skills-root "$temporary/bad-skills" >"$temporary/missing-worktree-preflight.log" 2>&1; then printf 'agent skill test error: missing worktree preflight unexpectedly passed\n' >&2; exit 1; fi
grep -Fq 'lacks worktree preflight behavior' "$temporary/missing-worktree-preflight.log"
rm -rf "$temporary/bad-skills"
cp -R "$repo_root/skills/." "$temporary/bad-skills"
sed 's/PR metadata preflight/PR creation review/' "$temporary/bad-skills/github-project-operator/SKILL.md" > "$temporary/missing-project-pr-preflight.md"
mv "$temporary/missing-project-pr-preflight.md" "$temporary/bad-skills/github-project-operator/SKILL.md"
if "$checker" --skills-root "$temporary/bad-skills" >"$temporary/missing-project-pr-preflight.log" 2>&1; then printf 'agent skill test error: missing project PR preflight unexpectedly passed\n' >&2; exit 1; fi
grep -Fq 'project operator lacks PR metadata preflight' "$temporary/missing-project-pr-preflight.log"
rm -rf "$temporary/bad-skills"
cp -R "$repo_root/skills/." "$temporary/bad-skills"
sed 's/PR metadata preflight/PR creation review/' "$temporary/bad-skills/github-documentation-architect/SKILL.md" > "$temporary/missing-docs-pr-preflight.md"
mv "$temporary/missing-docs-pr-preflight.md" "$temporary/bad-skills/github-documentation-architect/SKILL.md"
if "$checker" --skills-root "$temporary/bad-skills" >"$temporary/missing-docs-pr-preflight.log" 2>&1; then printf 'agent skill test error: missing documentation PR preflight unexpectedly passed\n' >&2; exit 1; fi
grep -Fq 'documentation architect lacks PR metadata preflight' "$temporary/missing-docs-pr-preflight.log"
"$checker" >/dev/null
printf 'repository agent skill tests passed\n'
