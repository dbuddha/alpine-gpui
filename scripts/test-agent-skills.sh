#!/bin/sh
set -eu
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT HUP INT TERM
mkdir "$fixture/example"
printf '%s\n' '---' 'name: example' 'description: Use for the example fixture.' '---' '[Reference](reference.md)' > "$fixture/example/SKILL.md"
: > "$fixture/example/reference.md"
scripts/check-agent-skills.sh --skills-root "$fixture" >/dev/null
rm "$fixture/example/reference.md"
if scripts/check-agent-skills.sh --skills-root "$fixture" > "$fixture/log" 2>&1; then
    echo 'missing reference accepted' >&2; exit 1
fi
grep -q 'missing reference' "$fixture/log"
: > "$fixture/example/reference.md"
mkdir "$fixture/duplicate"
cp "$fixture/example/SKILL.md" "$fixture/duplicate/SKILL.md"
if scripts/check-agent-skills.sh --skills-root "$fixture" > "$fixture/log" 2>&1; then
    echo 'duplicate name accepted' >&2; exit 1
fi
grep -q 'duplicate name' "$fixture/log"
rm -r "$fixture/duplicate"
sed '/^description:/d' "$fixture/example/SKILL.md" > "$fixture/new"
mv "$fixture/new" "$fixture/example/SKILL.md"
if scripts/check-agent-skills.sh --skills-root "$fixture" > "$fixture/log" 2>&1; then
    echo 'missing metadata accepted' >&2; exit 1
fi
grep -q 'missing description' "$fixture/log"
echo 'skill packaging fixtures passed'
