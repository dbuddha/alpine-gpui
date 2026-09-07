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
# Exercise the actual policy mode, not only the presence of prose in a skill.
# Snapshot values here are fixtures and do not attest to live GitHub state.
head=1111111111111111111111111111111111111111
base=2222222222222222222222222222222222222222
other=3333333333333333333333333333333333333333
"$checker" --merge-readiness manual main main protected "$head" "$head" "$base" "$base" success success 6 mergeable >/dev/null
"$checker" --merge-readiness manual main stack unprotected "$head" "$head" "$base" "$base" success success 6 mergeable >/dev/null
"$checker" --merge-readiness auto main main protected "$head" "$head" "$base" "$base" success success 6 mergeable >/dev/null
for defect in auto-stack auto-protected-stack auto-unprotected-default wrong-head wrong-base pending-aggregate failed-aggregate canceled-aggregate pending-checks failed-checks zero-checks invalid-count excessive-count unknown-protection unknown-mergeability conflict unknown-mode missing-branch malformed-head missing-base; do
    mode=manual; default_branch=main; base_branch=main; protection=protected
    run_head=$head; tested_base=$base; aggregate=success; required_checks=success
    selected_count=6; mergeability=mergeable
    case "$defect" in
        auto-stack) mode=auto; base_branch=stack; protection=unprotected ;;
        auto-protected-stack) mode=auto; base_branch=stack ;;
        auto-unprotected-default) mode=auto; protection=unprotected ;;
        wrong-head) run_head=$other ;;
        wrong-base) tested_base=$other ;;
        pending-aggregate) aggregate=pending ;;
        failed-aggregate) aggregate=failure ;;
        canceled-aggregate) aggregate=cancelled ;;
        pending-checks) required_checks=pending ;;
        failed-checks) required_checks=failure ;;
        zero-checks) selected_count=0 ;;
        invalid-count) selected_count=-1 ;;
        excessive-count) selected_count=10000 ;;
        unknown-protection) protection=unknown ;;
        unknown-mergeability) mergeability=unknown ;;
        conflict) mergeability=conflicting ;;
        unknown-mode) mode=force ;;
        missing-branch) default_branch= ;;
        malformed-head) run_head=not-a-revision ;;
        missing-base) tested_base=0000000000000000000000000000000000000000 ;;
    esac
    if "$checker" --merge-readiness "$mode" "$default_branch" "$base_branch" "$protection" "$head" "$run_head" "$base" "$tested_base" "$aggregate" "$required_checks" "$selected_count" "$mergeability" > "$temporary/merge-$defect.log" 2>&1; then
        printf 'agent skill test error: unsafe merge snapshot admitted: %s\n' "$defect" >&2; exit 1
    fi
    grep -Fq 'merge readiness policy error:' "$temporary/merge-$defect.log"
done
if "$checker" --merge-readiness manual > "$temporary/merge-incomplete.log" 2>&1; then
    printf 'agent skill test error: incomplete merge snapshot admitted\n' >&2; exit 1
fi
grep -Fq 'usage:' "$temporary/merge-incomplete.log"
fixture=$temporary/missing-merge-boundary
cp -R "$repo_root/skills" "$fixture"
sed 's/protected default branch/arbitrary target branch/g' "$fixture/github-project-operator/SKILL.md" > "$temporary/missing-merge-boundary.md"
mv "$temporary/missing-merge-boundary.md" "$fixture/github-project-operator/SKILL.md"
if "$checker" --skills-root "$fixture" > "$temporary/missing-merge-boundary.log" 2>&1; then
    printf 'agent skill test error: missing auto-merge protection boundary admitted\n' >&2; exit 1
fi
grep -Fq 'lacks the auto-merge protection boundary' "$temporary/missing-merge-boundary.log"
printf 'merge readiness policy controls passed: 3 accepted, 20 rejected, incomplete input, and missing guidance\n'
printf 'repository agent skill tests passed\n'
