#!/bin/sh
set -eu
repo_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
installer=$repo_root/scripts/install-agent-skills.sh
checker=$repo_root/scripts/check-agent-skills.sh
. "$repo_root/scripts/lib/agent-skills.sh"
skills=$(agent_skill_names "$repo_root" "$repo_root/skills")
temporary=$(mktemp -d "${TMPDIR:-/tmp}/alpine-agent-skills.XXXXXX")
cleanup() { find "$temporary" -depth -delete; }
trap cleanup EXIT HUP INT TERM
CODEX_HOME=$temporary/codex "$installer" --install >/dev/null
CODEX_HOME=$temporary/codex "$installer" --check >/dev/null
CODEX_HOME=$temporary/codex "$installer" --install >/dev/null
for skill in $skills; do [ -L "$temporary/codex/skills/$skill" ]; done
CODEX_HOME=$temporary/codex "$installer" --remove-links >/dev/null
for skill in $skills; do [ ! -e "$temporary/codex/skills/$skill" ]; done
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
sed 's/native blocked-by relationships/native dependency links/' "$temporary/bad-skills/github-project-operator/SKILL.md" > "$temporary/missing-native-dependencies.md"
mv "$temporary/missing-native-dependencies.md" "$temporary/bad-skills/github-project-operator/SKILL.md"
if "$checker" --skills-root "$temporary/bad-skills" >"$temporary/missing-native-dependencies.log" 2>&1; then printf 'agent skill test error: missing native dependency authority unexpectedly passed\n' >&2; exit 1; fi
grep -Fq 'project operator lacks native dependency authority' "$temporary/missing-native-dependencies.log"
rm -rf "$temporary/bad-skills"
cp -R "$repo_root/skills/." "$temporary/bad-skills"
sed 's/native issue hierarchy and dependencies/native issue planning/' "$temporary/bad-skills/github-documentation-architect/SKILL.md" > "$temporary/missing-reconciliation-order.md"
mv "$temporary/missing-reconciliation-order.md" "$temporary/bad-skills/github-documentation-architect/SKILL.md"
if "$checker" --skills-root "$temporary/bad-skills" >"$temporary/missing-reconciliation-order.log" 2>&1; then printf 'agent skill test error: missing documentation reconciliation order unexpectedly passed\n' >&2; exit 1; fi
grep -Fq 'documentation architect lacks reconciliation ordering' "$temporary/missing-reconciliation-order.log"
rm -rf "$temporary/bad-skills"
cp -R "$repo_root/skills/." "$temporary/bad-skills"
sed 's/PR metadata preflight/PR creation review/' "$temporary/bad-skills/github-documentation-architect/SKILL.md" > "$temporary/missing-docs-pr-preflight.md"
mv "$temporary/missing-docs-pr-preflight.md" "$temporary/bad-skills/github-documentation-architect/SKILL.md"
if "$checker" --skills-root "$temporary/bad-skills" >"$temporary/missing-docs-pr-preflight.log" 2>&1; then printf 'agent skill test error: missing documentation PR preflight unexpectedly passed\n' >&2; exit 1; fi
grep -Fq 'documentation architect lacks PR metadata preflight' "$temporary/missing-docs-pr-preflight.log"
# Every inventory member participates in whole-destination preflight, including
# a dangling foreign link. No earlier member may be partially installed.
for skill in $skills; do
    mkdir "$temporary/codex/skills/$skill"
    if CODEX_HOME=$temporary/codex "$installer" --install >"$temporary/all-foreign.log" 2>&1; then
        printf 'agent skill test error: foreign directory replaced: %s\n' "$skill" >&2; exit 1
    fi
    for member in $skills; do [ ! -L "$temporary/codex/skills/$member" ]; done
    rmdir "$temporary/codex/skills/$skill"
    ln -s "$temporary/foreign-target" "$temporary/codex/skills/$skill"
    if CODEX_HOME=$temporary/codex "$installer" --remove-links >"$temporary/all-foreign-remove.log" 2>&1; then
        printf 'agent skill test error: foreign link removed: %s\n' "$skill" >&2; exit 1
    fi
    [ "$(readlink "$temporary/codex/skills/$skill")" = "$temporary/foreign-target" ]
    unlink "$temporary/codex/skills/$skill"
done

for defect in duplicate malformed class folder evaluation header empty; do
    fixture=$temporary/manifest-$defect
    cp -R "$repo_root/skills" "$fixture"
    case "$defect" in
        duplicate) sed -n '2p' "$repo_root/skills/manifest.tsv" >> "$fixture/manifest.tsv" ;;
        malformed) awk 'BEGIN {FS=OFS="\t"} NR==2 {$1="../escape"} {print}' "$repo_root/skills/manifest.tsv" > "$fixture/manifest.tsv" ;;
        class) awk 'BEGIN {FS=OFS="\t"} NR==2 {$2="unknown"} {print}' "$repo_root/skills/manifest.tsv" > "$fixture/manifest.tsv" ;;
        folder) mv "$fixture/alpine-studio-gpui-engineer" "$temporary/parked-skill" ;;
        evaluation) awk 'BEGIN {FS=OFS="\t"} NR==2 {$3="assurance/agent-skills/v1/missing.tsv"} {print}' "$repo_root/skills/manifest.tsv" > "$fixture/manifest.tsv" ;;
        header) printf 'wrong\theader\n' > "$fixture/manifest.tsv" ;;
        empty) sed -n '1p' "$repo_root/skills/manifest.tsv" > "$fixture/manifest.tsv" ;;
    esac
    if "$checker" --skills-root "$fixture" > "$temporary/manifest-$defect.log" 2>&1; then
        printf 'agent skill test error: invalid manifest admitted: %s\n' "$defect" >&2; exit 1
    fi
    grep -Fq 'agent skill manifest error:' "$temporary/manifest-$defect.log"
done
"$checker" >/dev/null
printf 'repository agent skill tests passed\n'
