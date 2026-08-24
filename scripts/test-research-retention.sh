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

copy_fixture "$fixture_dir/missing-wiki-anchor"
sed '/issues\/175/d' \
    "$fixture_dir/missing-wiki-anchor/docs/research/index.md" \
    > "$fixture_dir/missing-wiki-anchor/catalog.md"
mv "$fixture_dir/missing-wiki-anchor/catalog.md" \
    "$fixture_dir/missing-wiki-anchor/docs/research/index.md"
if scripts/check-research-retention.sh "$fixture_dir/missing-wiki-anchor" \
    > "$fixture_dir/missing-wiki-anchor.log" 2>&1; then
    printf 'research retention test error: missing Wiki anchor unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'missing issue anchor #175' "$fixture_dir/missing-wiki-anchor.log"

copy_fixture "$fixture_dir/missing-wgpu-package"
rm "$fixture_dir/missing-wgpu-package/docs/research/wgpu/source-map.md"
if scripts/check-research-retention.sh "$fixture_dir/missing-wgpu-package" \
    > "$fixture_dir/missing-wgpu-package.log" 2>&1; then
    printf 'research retention test error: missing WGPU source map unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'required research artifact is missing' "$fixture_dir/missing-wgpu-package.log"

copy_fixture "$fixture_dir/missing-wgpu-pin"
sed 's/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/review-pin-removed/g' \
    "$fixture_dir/missing-wgpu-pin/docs/research/wgpu/index.md" \
    > "$fixture_dir/missing-wgpu-pin/wgpu-index.md"
mv "$fixture_dir/missing-wgpu-pin/wgpu-index.md" \
    "$fixture_dir/missing-wgpu-pin/docs/research/wgpu/index.md"
if scripts/check-research-retention.sh "$fixture_dir/missing-wgpu-pin" \
    > "$fixture_dir/missing-wgpu-pin.log" 2>&1; then
    printf 'research retention test error: missing WGPU pin unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'WGPU research is missing retained revision pin' "$fixture_dir/missing-wgpu-pin.log"

copy_fixture "$fixture_dir/missing-wgpu-classification"
sed 's/## Unverified hypotheses/## Open questions/' \
    "$fixture_dir/missing-wgpu-classification/docs/research/wgpu/findings.md" \
    > "$fixture_dir/missing-wgpu-classification/findings.md"
mv "$fixture_dir/missing-wgpu-classification/findings.md" \
    "$fixture_dir/missing-wgpu-classification/docs/research/wgpu/findings.md"
if scripts/check-research-retention.sh "$fixture_dir/missing-wgpu-classification" \
    > "$fixture_dir/missing-wgpu-classification.log" 2>&1; then
    printf 'research retention test error: missing WGPU evidence class unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'WGPU findings are missing evidence classification' \
    "$fixture_dir/missing-wgpu-classification.log"

copy_fixture "$fixture_dir/missing-post-baseline-pin"
sed '/7db5e18f6da8e02cd171668d4714c745c55d7eda/d' \
    "$fixture_dir/missing-post-baseline-pin/docs/research/alpine-lineage/source-map.md" \
    > "$fixture_dir/missing-post-baseline-pin/source-map.md"
mv "$fixture_dir/missing-post-baseline-pin/source-map.md" \
    "$fixture_dir/missing-post-baseline-pin/docs/research/alpine-lineage/source-map.md"
if scripts/check-research-retention.sh "$fixture_dir/missing-post-baseline-pin" \
    > "$fixture_dir/missing-post-baseline-pin.log" 2>&1; then
    printf 'research retention test error: missing post-baseline pin unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'lineage source map is missing retained revision pin' \
    "$fixture_dir/missing-post-baseline-pin.log"

copy_fixture "$fixture_dir/stale-current-state"
printf '\nOpen [#219](https://github.com/dbuddha/alpine-gpui/issues/219)\n' \
    >> "$fixture_dir/stale-current-state/docs/research/alpine-lineage/studio-lineage.md"
if scripts/check-research-retention.sh "$fixture_dir/stale-current-state" \
    > "$fixture_dir/stale-current-state.log" 2>&1; then
    printf 'research retention test error: stale current state unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'lineage package retains superseded current-state claim' \
    "$fixture_dir/stale-current-state.log"

printf 'research retention tests passed\n'
