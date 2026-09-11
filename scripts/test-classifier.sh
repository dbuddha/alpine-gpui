#!/bin/sh
set -eu

temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
classifier_program=${ALPINE_CLASSIFIER_UNDER_TEST:-$(pwd)/scripts/classify-ci.sh}

run_fixture() {
    output_file=$(mktemp "$temporary/output.XXXXXX")
    GITHUB_OUTPUT=$output_file \
    ALPINE_BASE_SHA=HEAD \
    ALPINE_HEAD_SHA=HEAD \
    ALPINE_CHANGED_FILES=$1 \
    ALPINE_PR_LABELS=${2:-} \
    "$classifier_program"
    cat "$output_file"
}

assert_output() {
    output=$1
    expected=$2
    if ! printf '%s\n' "$output" | grep -Fxq "$expected"; then
        printf 'classifier test error: expected %s\n%s\n' "$expected" "$output" >&2
        exit 1
    fi
}

assert_every_gate() {
    output=$1
    assert_output "$output" coverage=true
    assert_output "$output" mutation=true
    assert_output "$output" kani=true
    assert_output "$output" miri=true
    assert_output "$output" metal=true
    assert_output "$output" tla=true
    assert_output "$output" portable=true
}

docs=$(run_fixture README.md)
assert_output "$docs" coverage=false
assert_output "$docs" mutation=false
assert_output "$docs" kani=false
assert_output "$docs" tla=false
assert_output "$docs" portable=false

ci_workflow=$(run_fixture .github/workflows/ci.yml)
assert_every_gate "$ci_workflow"
assert_output "$ci_workflow" portable=true

nightly_workflow=$(run_fixture .github/workflows/nightly-assurance.yml)
assert_every_gate "$nightly_workflow"

release_workflow=$(run_fixture .github/workflows/release-dry-run.yml)
assert_every_gate "$release_workflow"

classifier=$(run_fixture scripts/classify-ci.sh)
assert_every_gate "$classifier"
assert_output "$classifier" portable=true

classifier_tests=$(run_fixture scripts/test-classifier.sh)
assert_every_gate "$classifier_tests"
assert_output "$classifier_tests" portable=true

kani_setup=$(run_fixture scripts/setup-kani.sh)
assert_every_gate "$kani_setup"

kani_setup_tests=$(run_fixture scripts/test-setup-kani.sh)
assert_every_gate "$kani_setup_tests"

studio_concurrency_stress=$(run_fixture scripts/test-studio-concurrency-stress.sh)
assert_every_gate "$studio_concurrency_stress"

coverage_checker=$(run_fixture scripts/check-coverage.sh)
assert_every_gate "$coverage_checker"

coverage_tests=$(run_fixture scripts/test-coverage.sh)
assert_every_gate "$coverage_tests"

miri_manifest=$(run_fixture assurance/miri-studio-partitions.tsv)
assert_every_gate "$miri_manifest"

miri_runner=$(run_fixture scripts/run-miri-partition.sh)
assert_every_gate "$miri_runner"

miri_tests=$(run_fixture scripts/test-miri-partitions.sh)
assert_every_gate "$miri_tests"

core=$(run_fixture crates/alpine-core/src/lib.rs)
assert_output "$core" coverage=true
assert_output "$core" mutation=true
assert_output "$core" kani=true
assert_output "$core" portable=true

text=$(run_fixture crates/alpine-text/src/lib.rs)
assert_output "$text" coverage=true
assert_output "$text" mutation=true
assert_output "$text" kani=true
assert_output "$text" metal=false

text_layout=$(run_fixture crates/alpine-text-layout/src/lib.rs)
assert_output "$text_layout" coverage=true
assert_output "$text_layout" mutation=true
assert_output "$text_layout" kani=true
assert_output "$text_layout" miri=true
assert_output "$text_layout" metal=false

studio=$(run_fixture apps/alpine-studio/src/lib.rs)
assert_output "$studio" coverage=true
assert_output "$studio" mutation=true
assert_output "$studio" kani=false
assert_output "$studio" metal=true

studio_manifest=$(run_fixture apps/alpine-studio/Cargo.toml)
assert_output "$studio_manifest" coverage=true
assert_output "$studio_manifest" mutation=true
assert_output "$studio_manifest" kani=false
assert_output "$studio_manifest" metal=true
assert_output "$studio_manifest" portable=true

studio_docs=$(run_fixture apps/alpine-studio/README.md)
assert_output "$studio_docs" coverage=false
assert_output "$studio_docs" mutation=false
assert_output "$studio_docs" kani=false

formal=$(run_fixture formal/tla/aep-0009/AssuranceLifecycle.tla)
assert_output "$formal" tla=true
assert_output "$formal" kani=false

qualification=$(run_fixture assurance/qualification/v1/valid.toml)
assert_output "$qualification" coverage=true
assert_output "$qualification" tla=true
assert_output "$qualification" mutation=true
assert_output "$qualification" kani=false

assurance=$(run_fixture tools/alpine-assurance/src/qualification.rs)
assert_output "$assurance" coverage=true
assert_output "$assurance" mutation=true
assert_output "$assurance" tla=true

trace=$(run_fixture tools/alpine-trace/src/lib.rs)
assert_output "$trace" coverage=true
assert_output "$trace" mutation=true
assert_output "$trace" kani=true
assert_output "$trace" tla=true

ax_client=$(run_fixture tools/alpine-ax-client/src/lib.rs)
assert_output "$ax_client" coverage=true
assert_output "$ax_client" mutation=true
assert_output "$ax_client" kani=false
assert_output "$ax_client" metal=true

ax_client_native=$(run_fixture tools/alpine-ax-client/src/native.rs)
assert_output "$ax_client_native" coverage=true
assert_output "$ax_client_native" mutation=true
assert_output "$ax_client_native" kani=false
assert_output "$ax_client_native" metal=true

ax_client_manifest=$(run_fixture tools/alpine-ax-client/Cargo.toml)
assert_output "$ax_client_manifest" coverage=true
assert_output "$ax_client_manifest" mutation=true
assert_output "$ax_client_manifest" kani=false
assert_output "$ax_client_manifest" metal=true

tool_docs=$(run_fixture tools/alpine-ax-client/README.md)
assert_output "$tool_docs" coverage=false
assert_output "$tool_docs" mutation=false

tool_fixture=$(run_fixture tools/alpine-ax-client/fixtures/tree.json)
assert_every_gate "$tool_fixture"

unsafe=$(run_fixture README.md review:unsafe)
assert_output "$unsafe" miri=true

metal=$(run_fixture crates/alpine-metal/src/lib.rs)
assert_output "$metal" coverage=true
assert_output "$metal" mutation=true
assert_output "$metal" kani=true
assert_output "$metal" metal=true

platform=$(run_fixture crates/alpine-platform/src/lib.rs)
assert_output "$platform" coverage=true
assert_output "$platform" mutation=true
assert_output "$platform" kani=true
assert_output "$platform" metal=false

macos_platform=$(run_fixture crates/alpine-platform-macos/src/native.rs)
assert_output "$macos_platform" coverage=true
assert_output "$macos_platform" mutation=true
assert_output "$macos_platform" kani=true
assert_output "$macos_platform" metal=true

macos_accessibility=$(run_fixture crates/alpine-platform-macos/src/native_accessibility.rs)
assert_output "$macos_accessibility" coverage=true
assert_output "$macos_accessibility" mutation=true
assert_output "$macos_accessibility" kani=true
assert_output "$macos_accessibility" metal=true

macos_signpost=$(run_fixture crates/alpine-platform-macos/src/signpost.rs)
assert_output "$macos_signpost" coverage=true
assert_output "$macos_signpost" mutation=true
assert_output "$macos_signpost" kani=true
assert_output "$macos_signpost" metal=true

shader=$(run_fixture shaders/offscreen.metal)
assert_output "$shader" coverage=false
assert_output "$shader" mutation=false
assert_output "$shader" kani=false
assert_output "$shader" metal=true
assert_output "$shader" portable=false

portable_checker=$(run_fixture scripts/check-portable-targets.sh)
assert_output "$portable_checker" portable=true

portable_tests=$(run_fixture scripts/test-portable-targets.sh)
assert_output "$portable_tests" portable=true

metal_gate=$(run_fixture scripts/check-metal.sh)
assert_output "$metal_gate" metal=true

native_benchmark_classifier=$(run_fixture scripts/check-native-benchmark-result.sh)
assert_output "$native_benchmark_classifier" metal=true

native_benchmark_classifier_tests=$(run_fixture scripts/test-native-benchmark-result.sh)
assert_output "$native_benchmark_classifier_tests" metal=true

for path in \
    crates/alpine-runtime/src/lib.rs \
    Cargo.toml Cargo.lock rust-toolchain.toml .cargo/config.toml \
    crates/alpine-core/Cargo.toml \
    .github/workflows/weekly-assurance.yml \
    .github/actions/upload-required-artifact/action.yml \
    assurance/miri-text-layout-partitions.tsv \
    scripts/check-native-mutation-receipts.sh \
    scripts/test-native-mutation-receipts.sh \
    scripts/check-tla.sh \
    apps/alpine-studio/fixtures/rust-analyzer/Cargo.toml \
    apps/alpine-studio/tests/fixtures/workspace/input.json \
    crates/alpine-core/fixtures/non-rust.bin \
    tools/unmapped-tool/src/lib.rs \
    tools/unmapped-tool/Cargo.toml \
    unclassified/input.bin; do
    assert_every_gate "$(run_fixture "$path")"
done

assert_every_gate "$(run_fixture "$(printf 'README.md\nunclassified/input.bin')")"
assert_every_gate "$(run_fixture "$(printf 'tools/alpine-assurance/src/main.rs\ntools/unmapped-tool/src/lib.rs\ntools/unmapped-tool/Cargo.toml')")"
empty=$(run_fixture '')
assert_output "$empty" mutation=false
assert_output "$empty" metal=false
assert_output "$empty" portable=false

# Invalid source identity must fail before publishing any workflow outputs.
for invalid in missing-base wrong-base wrong-head; do
    base=HEAD
    head=HEAD
    case "$invalid" in
        missing-base) base= ;;
        wrong-base) base=alpine-nonexistent-base ;;
        wrong-head) head=alpine-nonexistent-head ;;
    esac
    output_file="$temporary/$invalid.outputs"
    : > "$output_file"
    if GITHUB_OUTPUT="$output_file" ALPINE_BASE_SHA="$base" \
        ALPINE_HEAD_SHA="$head" ALPINE_CHANGED_FILES=README.md \
        "$classifier_program" > "$temporary/$invalid.stdout" 2> "$temporary/$invalid.stderr"; then
        printf 'classifier test error: accepted %s\n' "$invalid" >&2
        exit 1
    fi
    [ ! -s "$output_file" ] || {
        printf 'classifier test error: invalid identity published outputs\n' >&2
        exit 1
    }
    grep -q 'CI classifier error:' "$temporary/$invalid.stderr"
done

# Explain output is source-bound planning, not discovered or executed tests.
ALPINE_CI_PLAN="$temporary/plan.json" \
    run_fixture crates/alpine-runtime/src/lib.rs >/dev/null
source_head=$(git rev-parse HEAD)
jq -e --arg head "$source_head" '
    .schema == "alpine-ci-gate-plan/v1" and
    .head_sha == $head and .base_sha == $head and .merge_base == $head and
    .change_source == "fixture" and .gates.metal and .gates.mutation and
    .changed_paths == ["crates/alpine-runtime/src/lib.rs"] and
    .inventory_status == "not-discovered" and .acceptance == "not-evaluated" and
    any(.reasons[]; .gate == "metal" and .rule == "runtime-consumers")
' "$temporary/plan.json" >/dev/null

ALPINE_CI_PLAN="$temporary/unknown.json" \
    run_fixture "$(printf 'README.md\nunclassified/input.bin')" >/dev/null
jq -e '.unmapped_paths == ["unclassified/input.bin"] and
    any(.reasons[]; .gate == "metal" and .rule == "unmapped-input")' \
    "$temporary/unknown.json" >/dev/null

# Exercise real Git discovery, not only the injected path fixtures. No fixture
# repository touches the caller's index, branch, files, or Git configuration.
repository="$temporary/repository"
git init -q "$repository"
git -C "$repository" config user.name 'Alpine classifier fixture'
git -C "$repository" config user.email 'classifier@example.invalid'
git -C "$repository" config commit.gpgsign false
mkdir -p "$repository/crates/alpine-runtime/src" "$repository/docs"
printf 'initial\n' > "$repository/crates/alpine-runtime/src/lib.rs"
git -C "$repository" add .
git -C "$repository" commit -qm initial
initial=$(git -C "$repository" rev-parse HEAD)
printf 'runtime change\n' >> "$repository/crates/alpine-runtime/src/lib.rs"
git -C "$repository" add .
git -C "$repository" commit -qm runtime
printf 'documentation change\n' > "$repository/README.md"
git -C "$repository" add .
git -C "$repository" commit -qm documentation

run_git_fixture() {
    (
        cd "$repository"
        unset ALPINE_CHANGED_FILES GITHUB_OUTPUT ALPINE_PR_LABELS
        ALPINE_BASE_SHA=$1 ALPINE_HEAD_SHA=HEAD \
            ALPINE_CI_PLAN="$temporary/git-plan.json" "$classifier_program"
    )
}

assert_every_gate "$(run_git_fixture "$initial")"
jq -e '.change_source == "git" and
    (.changed_paths | index("crates/alpine-runtime/src/lib.rs") != null)' \
    "$temporary/git-plan.json" >/dev/null

before_rename=$(git -C "$repository" rev-parse HEAD)
git -C "$repository" mv crates/alpine-runtime/src/lib.rs docs/renamed.md
git -C "$repository" commit -qm rename
assert_every_gate "$(run_git_fixture "$before_rename")"
jq -e '(.changed_paths | index("crates/alpine-runtime/src/lib.rs") != null) and
    (.changed_paths | index("docs/renamed.md") != null)' \
    "$temporary/git-plan.json" >/dev/null

before_unusual=$(git -C "$repository" rev-parse HEAD)
unusual_path=$(printf 'docs/two\nlines.md')
printf 'unusual filename\n' > "$repository/$unusual_path"
git -C "$repository" add .
git -C "$repository" commit -qm unusual-path
assert_every_gate "$(run_git_fixture "$before_unusual")"
jq -e '(.unmapped_paths | length) == 1 and
    any(.reasons[]; .rule == "unmapped-input")' "$temporary/git-plan.json" >/dev/null

unchanged=$(run_git_fixture HEAD)
assert_output "$unchanged" coverage=false
assert_output "$unchanged" mutation=false
assert_output "$unchanged" portable=false
jq -e '.changed_paths == [] and .reasons == []' "$temporary/git-plan.json" >/dev/null

# Dispatch is an explicit comparison, not an implicit one-commit fallback.
# Its supplied baseline must include earlier runtime changes as well as the
# most recent documentation change. Invalid/missing bases fail above for all
# events; marking a command as dispatch cannot make either identity valid.
dispatch=$(GITHUB_EVENT_NAME=workflow_dispatch run_git_fixture "$initial")
assert_every_gate "$dispatch"
jq -e --arg base "$initial" '.base_sha == $base and .change_source == "git"' \
    "$temporary/git-plan.json" >/dev/null
if GITHUB_EVENT_NAME=workflow_dispatch run_git_fixture '' \
    > "$temporary/dispatch.stdout" 2> "$temporary/dispatch.stderr"; then
    printf 'classifier test error: dispatch accepted a missing baseline\n' >&2
    exit 1
fi
grep -q 'ALPINE_BASE_SHA is required' "$temporary/dispatch.stderr"

# Unrelated histories cannot authorize a partial diff or an empty plan.
foreign=$(printf 'foreign root\n' | git -C "$repository" commit-tree \
    "$(git -C "$repository" rev-parse 'HEAD^{tree}')")
if run_git_fixture "$foreign" > "$temporary/foreign.stdout" 2> "$temporary/foreign.stderr"; then
    printf 'classifier test error: accepted unrelated histories\n' >&2
    exit 1
fi
grep -q 'no available common ancestor' "$temporary/foreign.stderr"

printf 'CI classifier tests passed\n'
