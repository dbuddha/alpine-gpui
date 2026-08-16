#!/bin/sh
set -eu

fixture_dir=$(mktemp -d)
trap 'rm -rf "$fixture_dir"' EXIT HUP INT TERM

copy_fixture() {
    destination=$1
    mkdir -p "$destination"
    cp -R docs "$destination/docs"
}

copy_fixture "$fixture_dir/valid"
scripts/check-research-retention.sh "$fixture_dir/valid" >/dev/null

copy_fixture "$fixture_dir/broken-link"
printf '\n[Missing local evidence](missing-evidence.md)\n' \
    >> "$fixture_dir/broken-link/docs/research/index.md"
if scripts/check-research-retention.sh "$fixture_dir/broken-link" \
    > "$fixture_dir/broken-link.log" 2>&1; then
    printf 'research retention test error: broken link unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'repository-relative research links do not resolve' "$fixture_dir/broken-link.log"

copy_fixture "$fixture_dir/missing-anchor"
sed '/issues\/32/d' \
    "$fixture_dir/missing-anchor/docs/research/alpine-studio-adversarial-review.md" \
    > "$fixture_dir/missing-anchor/review.md"
mv "$fixture_dir/missing-anchor/review.md" \
    "$fixture_dir/missing-anchor/docs/research/alpine-studio-adversarial-review.md"
if scripts/check-research-retention.sh "$fixture_dir/missing-anchor" \
    > "$fixture_dir/missing-anchor.log" 2>&1; then
    printf 'research retention test error: missing requirement anchor unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'missing research anchor for Requirement #32' "$fixture_dir/missing-anchor.log"

copy_fixture "$fixture_dir/missing-environment"
sed 's/environment_hash/environment_identity/g' \
    "$fixture_dir/missing-environment/docs/quality/comparator-protocol.md" \
    > "$fixture_dir/missing-environment/comparator.md"
mv "$fixture_dir/missing-environment/comparator.md" \
    "$fixture_dir/missing-environment/docs/quality/comparator-protocol.md"
if scripts/check-research-retention.sh "$fixture_dir/missing-environment" \
    > "$fixture_dir/missing-environment.log" 2>&1; then
    printf 'research retention test error: missing environment identity unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'missing mandatory field environment_hash' "$fixture_dir/missing-environment.log"

copy_fixture "$fixture_dir/missing-exclusions"
sed 's/exclusion_manifest_hash/exclusion_identity/g' \
    "$fixture_dir/missing-exclusions/docs/quality/comparator-protocol.md" \
    > "$fixture_dir/missing-exclusions/comparator.md"
mv "$fixture_dir/missing-exclusions/comparator.md" \
    "$fixture_dir/missing-exclusions/docs/quality/comparator-protocol.md"
if scripts/check-research-retention.sh "$fixture_dir/missing-exclusions" \
    > "$fixture_dir/missing-exclusions.log" 2>&1; then
    printf 'research retention test error: missing exclusion identity unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'missing mandatory field exclusion_manifest_hash' "$fixture_dir/missing-exclusions.log"

printf 'research retention tests passed\n'
