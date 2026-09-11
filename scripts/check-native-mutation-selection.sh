#!/bin/sh
# Policy witness only: no mutant discovery or execution acceptance.
set -eu
fail() { printf 'native mutation selection: %s\n' "$1" >&2; exit 1; }
[ "${ALPINE_CHANGED_FILES+x}" != x ] || fail 'fixture injection is forbidden'
case "${NATIVE_SELECTION_MODE-}" in full|affected) ;; *) fail 'invalid selection mode' ;; esac
case "${NATIVE_MUTATION_REQUIRED-}" in true|false) ;; *) fail 'invalid mutation requirement' ;; esac
case "${METAL_REQUIRED-}" in true|false) ;; *) fail 'invalid metal requirement' ;; esac
base=${ALPINE_BASE_SHA-} head=${ALPINE_HEAD_SHA-} tested=${GITHUB_SHA-}
event_base=${ALPINE_EVENT_BASE_SHA-} event_head=${ALPINE_EVENT_HEAD_SHA-}
for sha in "$base" "$head" "$tested" "$event_base" "$event_head"; do
    case "$sha" in ''|*[!0-9a-f]*) fail 'invalid commit SHA' ;; esac
    [ "${#sha}" -eq 40 ] || fail 'invalid commit SHA'
    resolved=$(git rev-parse --verify --end-of-options "$sha^{commit}") || fail 'unavailable commit'
    [ "$resolved" = "$sha" ] || fail 'commit identity mismatch'
done
[ "$base" = "$event_base" ] || fail 'base differs from event base'
[ "$head" = "$event_head" ] || fail 'head differs from event head'
[ "$(git rev-parse --show-prefix)" = '' ] || fail 'run from repository root'
[ "$(git rev-parse HEAD)" = "$tested" ] || fail 'checkout differs from GITHUB_SHA'
if [ "$tested" != "$head" ]; then
    [ "$(git show -s --format=%P "$tested")" = "$base $head" ] || fail 'checkout is not head or exact base/head merge'
fi
merge_base=$(git merge-base "$base" "$head") || fail 'no common ancestor'
tree=$(git rev-parse 'HEAD^{tree}')
required=false
if [ "$METAL_REQUIRED" = true ] && [ "$NATIVE_SELECTION_MODE" = full ]; then required=true; fi
[ "$NATIVE_MUTATION_REQUIRED" = "$required" ] || fail 'contradictory mutation requirement'
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
classifier=scripts/classify-ci.sh workflow=.github/workflows/ci.yml
for source in "$classifier" "$workflow"; do
    [ -f "$source" ] && [ ! -L "$source" ] || fail 'missing or linked policy source'
    git show "$tested:$source" > "$temporary/bound" || fail 'untracked policy source'
    cmp -s "$source" "$temporary/bound" || fail 'policy source differs from tested commit'
done
GITHUB_OUTPUT= ALPINE_CI_PLAN= sh "$classifier" > "$temporary/outputs" || fail 'classification failed'
expect() {
    count=$(grep -c "^$1=" "$temporary/outputs" || :)
    [ "$count" = 1 ] && grep -Fxq "$1=$2" "$temporary/outputs" || fail "classifier contradicts $1"
}
expect base_sha "$base"
expect head_sha "$head"
expect native_selection "$NATIVE_SELECTION_MODE"
expect native_mutation_required "$NATIVE_MUTATION_REQUIRED"
expect metal "$METAL_REQUIRED"
git diff --no-ext-diff --no-textconv --no-renames "$base...$head" -- > "$temporary/source.diff"
hash() {
    if command -v sha256sum >/dev/null 2>&1; then value=$(sha256sum "$1"); else value=$(shasum -a 256 "$1"); fi
    printf '%s\n' "${value%% *}"
}
jq -n --arg base "$base" --arg head "$head" --arg tested "$tested" --arg tree "$tree" \
    --arg merge_base "$merge_base" --arg mode "$NATIVE_SELECTION_MODE" --arg labels "${ALPINE_PR_LABELS-}" \
    --argjson required "$required" --argjson metal "$METAL_REQUIRED" \
    --arg diff "$(hash "$temporary/source.diff")" --arg classifier "$(hash "$classifier")" \
    --arg workflow "$(hash "$workflow")" \
    '{schema:"alpine-native-mutation-policy/v1",base:$base,head:$head,merge_base:$merge_base,
      tested_commit:$tested,tested_tree:$tree,mode:$mode,pr_labels:$labels,
      native_mutation_required:$required,metal_required:$metal,diff_sha256:$diff,
      classifier_sha256:$classifier,workflow_sha256:$workflow,
      scope:"native-mutation-policy-only",inventory:"not-discovered",execution:"not-evaluated"}' > "$temporary/witness"
witness=${ALPINE_NATIVE_SELECTION_WITNESS:-target/native-mutation-policy.json}
mkdir -p "$(dirname "$witness")"
(set -C; cat "$temporary/witness" > "$witness") || fail 'cannot create fresh policy witness'
printf 'native mutation policy witness written: %s\n' "$witness"
