#!/bin/sh
set -eu

fail() {
    printf 'CI classifier error: %s\n' "$1" >&2
    exit 2
}

base_ref=${ALPINE_BASE_SHA:-}
head_ref=${ALPINE_HEAD_SHA:-HEAD}
labels=${ALPINE_PR_LABELS:-}
[ -n "$base_ref" ] || fail 'ALPINE_BASE_SHA is required; a one-commit fallback is not a PR baseline'
base_sha=$(git rev-parse --verify --end-of-options "$base_ref^{commit}" 2>/dev/null) ||
    fail 'base does not identify an available commit'
head_sha=$(git rev-parse --verify --end-of-options "$head_ref^{commit}" 2>/dev/null) ||
    fail 'head does not identify an available commit'
merge_base=$(git merge-base "$base_sha" "$head_sha" 2>/dev/null) ||
    fail 'base and head have no available common ancestor'

change_source=git
if [ "${ALPINE_CHANGED_FILES+x}" = x ]; then
    # Explicit fixture input is never represented as discovered Git evidence.
    changed_files=$ALPINE_CHANGED_FILES
    change_source=fixture
else
    # Disabling rename detection includes both ownership endpoints. Git quotes
    # unusual path bytes, including newlines; those select the fallback below.
    changed_files=$(git -c core.quotepath=true diff --no-renames --name-only \
        "$merge_base" "$head_sha" --) || fail 'changed-path discovery failed'
fi

temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
: > "$temporary/reasons.tsv"

matches() {
    printf '%s\n' "$changed_files" | grep -Eq "$1"
}

has_label() {
    printf '%s\n' "$labels" | tr ',' '\n' | grep -Fxq "$1"
}

coverage=false
mutation=false
kani=false
miri=false
metal=false
tla=false
portable=false

enable() {
    reason=$1
    shift
    for gate in "$@"; do
        case "$gate" in
            coverage) coverage=true ;;
            mutation) mutation=true ;;
            kani) kani=true ;;
            miri) miri=true ;;
            metal) metal=true ;;
            tla) tla=true ;;
            portable) portable=true ;;
            *) fail 'internal unknown gate' ;;
        esac
        printf '%s\t%s\n' "$gate" "$reason" >> "$temporary/reasons.tsv"
    done
}

enable_all() {
    enable "$1" coverage mutation kani miri metal tla portable
}

# This first recovery slice only broadens selection. A control plane must not
# classify out the evidence needed to establish its own behavior.
if matches '^(\.github/(workflows/.+\.ya?ml$|actions/)|assurance/miri-[^/]+\.tsv$|scripts/(classify-ci|test-classifier|setup-kani|test-setup-kani|test-studio-concurrency-stress|check-coverage|test-coverage|run-miri-partition|test-miri-partitions|check-native-mutation-receipts|test-native-mutation-receipts|check-tla|test-formal-effectiveness)\.sh$)'; then
    enable_all ci-control-plane
fi

# Shared configuration invalidates more than portable compilation. Per-package
# manifests are covered conservatively until feature/build edges are explicit.
if matches '^(Cargo\.(toml|lock)$|rust-toolchain(\.toml)?$|\.cargo/|crates/[^/]+/Cargo\.toml$)'; then
    enable_all shared-build-input
fi

if matches '^(Cargo\.toml$|Cargo\.lock$|crates/.+\.rs$|crates/.+/Cargo\.toml$|apps/.+\.rs$|apps/.+/Cargo\.toml$|tools/[^/]+/(src/.+\.rs|Cargo\.toml)$|tools/alpine-trace/)'; then
    enable rust-implementation coverage
fi

if matches '^(crates/(alpine-core|alpine-scene|alpine-renderer|alpine-metal|alpine-platform|alpine-platform-macos|alpine-text|alpine-text-layout)/.+\.rs$|tools/alpine-trace/.+\.rs$)'; then
    enable critical-rust mutation kani
fi

# Runtime dispatch reaches Studio and the native owner even without edits in
# either consumer. This is conservative pending the dependency-aware graph.
if matches '^crates/alpine-runtime/'; then
    enable_all runtime-consumers
fi

if matches '^(apps/alpine-studio/.+\.rs$|apps/alpine-studio/Cargo\.toml$)'; then
    enable studio-rust mutation
fi

if matches '^tools/[^/]+/(src/.+\.rs|Cargo\.toml)$'; then
    enable tool-rust mutation
fi

if matches '^(tools/alpine-assurance/.+\.rs$|assurance/qualification/)'; then
    enable assurance-implementation coverage mutation
fi

if matches '^(formal/tla/|docs/aep/|assurance/evidence\.toml$|assurance/qualification/|tools/alpine-assurance/|tools/alpine-trace/)'; then
    enable formal-contract tla
fi

if has_label review:unsafe || matches '^(crates/alpine-text-layout/|crates/.+/(unsafe|ffi|resource|lifetime))'; then
    enable unsafe-or-lifetime miri
fi

if matches '^(apps/alpine-studio/(.+\.rs|Cargo\.toml)$|crates/(alpine-metal|alpine-platform-macos)/|tools/alpine-ax-client/(src/.+\.rs|Cargo\.toml)$|shaders/|.+\.metal$)'; then
    enable native-rendering metal
fi

# Non-Rust inputs participate in production tests. Until their individual
# inventories are mapped, keep all native and deterministic consumers selected.
if matches '^(apps/alpine-studio|tools/alpine-ax-client)/(fixtures|tests|assets|resources)/'; then
    enable_all native-test-input
fi

if matches '^scripts/(check-metal|check-native-benchmark-result|test-native-benchmark-result)\.sh$'; then
    enable metal-orchestration metal
fi

if matches '(^|/)(Cargo\.toml|build\.rs|[^/]+\.rs)$|^(Cargo\.lock|rust-toolchain\.toml|\.github/workflows/.+\.yml|scripts/(check-portable-targets|test-portable-targets|classify-ci|test-classifier)\.sh)$'; then
    enable portable-compilation portable
fi

# An explicit known-input list prevents a mapped file from masking an unknown
# consumer in the same diff. Generic Rust/manifest matches above are not proof
# that an otherwise unrecognized package or input has complete gate coverage.
known_inputs='^(README\.md$|ARCHITECTURE\.md$|AGENTS\.md$|CONTRIBUTING\.md$|CHANGELOG\.md$|LICENSE([^/]*$)|NOTICE([^/]*$)|book\.toml$|docs/|skills/|\.github/(ISSUE_TEMPLATE/|pull_request_template\.md$|workflows/.+\.ya?ml$|actions/)|Cargo\.(toml|lock)$|rust-toolchain(\.toml)?$|\.cargo/|crates/(alpine-core|alpine-scene|alpine-renderer|alpine-metal|alpine-platform|alpine-platform-macos|alpine-text|alpine-text-layout|alpine-runtime)/(.+\.rs$|Cargo\.toml$|README\.md$)|apps/alpine-studio/(.+\.rs$|Cargo\.toml$|README\.md$|fixtures/|tests/|assets/|resources/)|tools/(alpine-assurance|alpine-trace|alpine-ax-client)/(src/.+\.rs$|Cargo\.toml$|README\.md$)|tools/alpine-(trace|assurance)/|tools/alpine-ax-client/(fixtures|tests|assets|resources)/|formal/tla/|assurance/(evidence\.toml$|qualification/|miri-[^/]+\.tsv$)|shaders/|.+\.metal$|scripts/(classify-ci|test-classifier|setup-kani|test-setup-kani|test-studio-concurrency-stress|check-coverage|test-coverage|run-miri-partition|test-miri-partitions|check-native-mutation-receipts|test-native-mutation-receipts|check-tla|test-formal-effectiveness|check-metal|check-native-benchmark-result|test-native-benchmark-result|check-portable-targets|test-portable-targets)\.sh$)'
unknown_inputs=$(printf '%s\n' "$changed_files" | sed '/^$/d' | grep -Ev "$known_inputs") || {
    result=$?
    [ "$result" -eq 1 ] || fail 'known-input classification failed'
}
if [ -n "$unknown_inputs" ]; then
    enable_all unmapped-input
fi

# Explanations are optional planning artifacts, not test execution or acceptance
# receipts. Existing workflow outputs retain their names and boolean values.
if [ -n "${ALPINE_CI_PLAN:-}" ]; then
    command -v jq >/dev/null 2>&1 || fail 'jq is required for ALPINE_CI_PLAN'
    jq -n \
        --arg base "$base_sha" --arg head "$head_sha" --arg merge_base "$merge_base" \
        --arg change_source "$change_source" --arg paths "$changed_files" \
        --arg labels "$labels" --arg unknown "$unknown_inputs" \
        --rawfile reasons "$temporary/reasons.tsv" \
        --argjson coverage "$coverage" --argjson mutation "$mutation" \
        --argjson kani "$kani" --argjson miri "$miri" --argjson metal "$metal" \
        --argjson tla "$tla" --argjson portable "$portable" \
        '{schema: "alpine-ci-gate-plan/v1", base_sha: $base, head_sha: $head,
          merge_base: $merge_base, change_source: $change_source,
          changed_paths: ($paths | split("\n") | map(select(length > 0))),
          risk_labels: ($labels | gsub(","; "\n") | split("\n") | map(select(length > 0))),
          unmapped_paths: ($unknown | split("\n") | map(select(length > 0))),
          gates: {coverage: $coverage, mutation: $mutation, kani: $kani,
                  miri: $miri, metal: $metal, tla: $tla, portable: $portable},
          reasons: ($reasons | split("\n") | map(select(length > 0) | split("\t") |
                    {gate: .[0], rule: .[1]}) | unique),
          inventory_status: "not-discovered", acceptance: "not-evaluated",
          scope: "optional-gate-selection-only"}' > "$temporary/plan.json"
    mv "$temporary/plan.json" "$ALPINE_CI_PLAN"
fi

{
    printf 'base_sha=%s\n' "$base_sha"
    printf 'head_sha=%s\n' "$head_sha"
    printf 'coverage=%s\n' "$coverage"
    printf 'mutation=%s\n' "$mutation"
    printf 'kani=%s\n' "$kani"
    printf 'miri=%s\n' "$miri"
    printf 'metal=%s\n' "$metal"
    printf 'tla=%s\n' "$tla"
    printf 'portable=%s\n' "$portable"
} > "$temporary/outputs"
if [ -n "${GITHUB_OUTPUT:-}" ]; then
    cat "$temporary/outputs" >> "$GITHUB_OUTPUT"
else
    cat "$temporary/outputs"
fi
