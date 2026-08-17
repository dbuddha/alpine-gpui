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
"$checker" >/dev/null
printf 'repository agent skill tests passed\n'
