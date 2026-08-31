#!/bin/sh
set -eu

run_fixture() {
    output_file=$(mktemp)
    GITHUB_OUTPUT=$output_file \
    ALPINE_BASE_SHA=HEAD \
    ALPINE_HEAD_SHA=HEAD \
    ALPINE_CHANGED_FILES=$1 \
    ALPINE_PR_LABELS=${2:-} \
    scripts/classify-ci.sh
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
}

docs=$(run_fixture README.md)
assert_output "$docs" coverage=false
assert_output "$docs" mutation=false
assert_output "$docs" kani=false
assert_output "$docs" tla=false

ci_workflow=$(run_fixture .github/workflows/ci.yml)
assert_every_gate "$ci_workflow"

nightly_workflow=$(run_fixture .github/workflows/nightly-assurance.yml)
assert_every_gate "$nightly_workflow"

release_workflow=$(run_fixture .github/workflows/release-dry-run.yml)
assert_every_gate "$release_workflow"

classifier=$(run_fixture scripts/classify-ci.sh)
assert_every_gate "$classifier"

classifier_tests=$(run_fixture scripts/test-classifier.sh)
assert_every_gate "$classifier_tests"

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
assert_output "$ax_client" metal=false

ax_client_native=$(run_fixture tools/alpine-ax-client/src/native.rs)
assert_output "$ax_client_native" coverage=true
assert_output "$ax_client_native" mutation=true
assert_output "$ax_client_native" kani=false
assert_output "$ax_client_native" metal=false

ax_client_manifest=$(run_fixture tools/alpine-ax-client/Cargo.toml)
assert_output "$ax_client_manifest" coverage=true
assert_output "$ax_client_manifest" mutation=true
assert_output "$ax_client_manifest" kani=false
assert_output "$ax_client_manifest" metal=false

tool_docs=$(run_fixture tools/alpine-ax-client/README.md)
assert_output "$tool_docs" coverage=false
assert_output "$tool_docs" mutation=false

tool_fixture=$(run_fixture tools/alpine-ax-client/fixtures/tree.json)
assert_output "$tool_fixture" coverage=false
assert_output "$tool_fixture" mutation=false

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

metal_gate=$(run_fixture scripts/check-metal.sh)
assert_output "$metal_gate" metal=true

native_benchmark_classifier=$(run_fixture scripts/check-native-benchmark-result.sh)
assert_output "$native_benchmark_classifier" metal=true

native_benchmark_classifier_tests=$(run_fixture scripts/test-native-benchmark-result.sh)
assert_output "$native_benchmark_classifier_tests" metal=true

printf 'CI classifier tests passed\n'
