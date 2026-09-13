#!/bin/sh
set -eu

# Shared control entrypoint for hosted quality and the canonical local check.
scripts/test-native-command.sh

fixture_dir=$(mktemp -d)
trap 'rm -rf "$fixture_dir"' EXIT HUP INT TERM

# Policy must work without issues, claims, labels or any GitHub access.
cat > "$fixture_dir/gh" <<'EOF'
#!/bin/sh
printf 'unexpected GitHub access\n' >&2
exit 99
EOF
chmod +x "$fixture_dir/gh"
run_policy() {
    PATH="$fixture_dir:$PATH" \
    GITHUB_EVENT_NAME=pull_request GITHUB_REPOSITORY=dbuddha/alpine-gpui \
    ALPINE_PR_BODY= ALPINE_PR_TITLE= ALPINE_PR_LABELS= \
    scripts/check-policy.sh
}
for source in crates/alpine-core/src/lib.rs apps/alpine-editor/src/lib.rs README.md; do
    ALPINE_CHANGED_FILES="$source" run_policy >/dev/null
done
( LC_ALL=en_US.UTF-8 run_policy >/dev/null )

for job in preflight quality native; do
    awk -v job="$job" '
        /^  [A-Za-z0-9_-]+:/ { selected = ($0 == "  " job ":") }
        selected && /^    runs-on:/ { $0 = "    runs-on: ubuntu-24.04" }
        { print }
    ' .github/workflows/ci.yml > "$fixture_dir/non-macos-$job.yml"
    if ALPINE_CI_WORKFLOW="$fixture_dir/non-macos-$job.yml" run_policy \
        > "$fixture_dir/non-macos-$job.log" 2>&1; then
        printf 'policy test error: product job %s accepted a non-macOS host\n' "$job" >&2
        exit 1
    fi
    grep -Fq "CI product job $job must use Apple Silicon macOS" "$fixture_dir/non-macos-$job.log"
    unset ALPINE_CI_WORKFLOW
done

# POSIX shell-function prefix assignments persist. Negative overrides must
# not contaminate the next positive policy check or a different gate family.
run_policy >/dev/null

cp .github/workflows/ci.yml "$fixture_dir/ci.yml"
ALPINE_CI_WORKFLOW="$fixture_dir/ci.yml" run_policy >/dev/null

cp .github/actions/upload-required-artifact/action.yml "$fixture_dir/upload-required-artifact.yml"
ALPINE_REQUIRED_ARTIFACT_ACTION="$fixture_dir/upload-required-artifact.yml" run_policy >/dev/null


sed 's#actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a#actions/upload-artifact@0000000000000000000000000000000000000000#' \
    "$fixture_dir/upload-required-artifact.yml" > "$fixture_dir/unpinned-required-artifact.yml"
if ALPINE_REQUIRED_ARTIFACT_ACTION="$fixture_dir/unpinned-required-artifact.yml" \
    run_policy > "$fixture_dir/unpinned-required-artifact.log" 2>&1; then
    printf 'policy test error: unpinned required artifact helper unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'required artifact helper must retain one tolerated primary and one identical blocking failure-only retry' \
    "$fixture_dir/unpinned-required-artifact.log"; then
    printf 'policy test error: expected required artifact pin failure was not reported\n' >&2
    cat "$fixture_dir/unpinned-required-artifact.log" >&2
    exit 1
fi

sed "s/steps.primary.outcome == 'failure'/steps.primary.outcome == 'success'/" \
    "$fixture_dir/upload-required-artifact.yml" > "$fixture_dir/wrong-required-artifact-route.yml"
if ALPINE_REQUIRED_ARTIFACT_ACTION="$fixture_dir/wrong-required-artifact-route.yml" \
    run_policy > "$fixture_dir/wrong-required-artifact-route.log" 2>&1; then
    printf 'policy test error: incorrectly routed required artifact retry unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'required artifact helper must retain one tolerated primary and one identical blocking failure-only retry' \
    "$fixture_dir/wrong-required-artifact-route.log"; then
    printf 'policy test error: expected required artifact routing failure was not reported\n' >&2
    cat "$fixture_dir/wrong-required-artifact-route.log" >&2
    exit 1
fi

sed "/steps.primary.outcome == 'failure'/a\\
      continue-on-error: true" "$fixture_dir/upload-required-artifact.yml" \
    > "$fixture_dir/nonblocking-required-artifact-retry.yml"
if ALPINE_REQUIRED_ARTIFACT_ACTION="$fixture_dir/nonblocking-required-artifact-retry.yml" \
    run_policy > "$fixture_dir/nonblocking-required-artifact-retry.log" 2>&1; then
    printf 'policy test error: nonblocking required artifact retry unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'required artifact helper must retain one tolerated primary and one identical blocking failure-only retry' \
    "$fixture_dir/nonblocking-required-artifact-retry.log"; then
    printf 'policy test error: expected blocking required artifact retry failure was not reported\n' >&2
    cat "$fixture_dir/nonblocking-required-artifact-retry.log" >&2
    exit 1
fi

sed 's/name: ${{ inputs.name }}/name: drift/' "$fixture_dir/upload-required-artifact.yml" \
    > "$fixture_dir/drifted-required-artifact-contract.yml"
if ALPINE_REQUIRED_ARTIFACT_ACTION="$fixture_dir/drifted-required-artifact-contract.yml" \
    run_policy > "$fixture_dir/drifted-required-artifact-contract.log" 2>&1; then
    printf 'policy test error: drifted required artifact forwarding unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'required artifact helper must retain one tolerated primary and one identical blocking failure-only retry' \
    "$fixture_dir/drifted-required-artifact-contract.log"; then
    printf 'policy test error: expected required artifact forwarding failure was not reported\n' >&2
    cat "$fixture_dir/drifted-required-artifact-contract.log" >&2
    exit 1
fi
unset ALPINE_REQUIRED_ARTIFACT_ACTION

perl -0pe 's#uses: \Q./.github/actions/upload-required-artifact\E#uses: actions/upload-artifact\@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a#' \

sed 's/types: \[opened,/types: [edited, opened,/' \
    "$fixture_dir/ci.yml" > "$fixture_dir/opened-pr-fanout-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/opened-pr-fanout-ci.yml" run_policy > "$fixture_dir/opened-pr-fanout-ci.log" 2>&1; then
    printf 'policy test error: opened PR fan-out unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'CI pull_request triggers must include opened, synchronize and reopened only' \
    "$fixture_dir/opened-pr-fanout-ci.log"; then
    printf 'policy test error: expected settled PR trigger failure was not reported\n' >&2
    cat "$fixture_dir/opened-pr-fanout-ci.log" >&2
    exit 1
fi

sed 's/^  group: .*/  group: ci-superseded-${{ github.run_id }}/' \
    "$fixture_dir/ci.yml" > "$fixture_dir/shared-metadata-concurrency-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/shared-metadata-concurrency-ci.yml" run_policy > "$fixture_dir/shared-metadata-concurrency-ci.log" 2>&1; then
    printf 'policy test error: per-run metadata concurrency unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'CI events for one ref must share a concurrency group' \
    "$fixture_dir/shared-metadata-concurrency-ci.log"; then
    printf 'policy test error: expected metadata concurrency failure was not reported\n' >&2
    cat "$fixture_dir/shared-metadata-concurrency-ci.log" >&2
    exit 1
fi

sed 's/^  cancel-in-progress: .*/  cancel-in-progress: false/' \
    "$fixture_dir/ci.yml" > "$fixture_dir/unconditional-pr-cancellation-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/unconditional-pr-cancellation-ci.yml" run_policy > "$fixture_dir/unconditional-pr-cancellation-ci.log" 2>&1; then
    printf 'policy test error: disabled superseded-run cancellation unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'CI must cancel superseded runs for the same ref' \
    "$fixture_dir/unconditional-pr-cancellation-ci.log"; then
    printf 'policy test error: expected superseded-run cancellation failure was not reported\n' >&2
    cat "$fixture_dir/unconditional-pr-cancellation-ci.log" >&2
    exit 1
fi

for admission_fault in fast-feedback native-command native-cfg native-ignored; do
    case "$admission_fault" in
        fast-feedback)
            expression='s/run: scripts\/check-ci-fast-feedback\.sh/run: true/'
            diagnostic='CI preflight must validate technical repository policy before fan-out'
            ;;
        native-command)
            expression='s/run: scripts\/check-ci-native-admission\.sh/run: true/'
            diagnostic='CI native mutation admission must preserve the explicit native baseline contract'
            ;;
        native-cfg)
            expression='s/(Verify native execution.*?RUSTFLAGS:) --cfg alpine_native_validation/$1 ordinary/s'
            diagnostic='CI native mutation admission must preserve the explicit native baseline contract'
            ;;
        native-ignored)
            expression='s/(        run: scripts\/check-ci-native-admission\.sh)/        continue-on-error: true\n$1/'
            diagnostic='CI native mutation admission must not ignore baseline failures'
            ;;
    esac
    perl -0pe "$expression" "$fixture_dir/ci.yml" > "$fixture_dir/$admission_fault-ci.yml"
    if ALPINE_CI_WORKFLOW="$fixture_dir/$admission_fault-ci.yml" run_policy > "$fixture_dir/$admission_fault-ci.log" 2>&1; then
        printf 'policy test error: admission fault %s unexpectedly passed\n' "$admission_fault" >&2
        exit 1
    fi
    if ! grep -Fq "$diagnostic" "$fixture_dir/$admission_fault-ci.log"; then
        printf 'policy test error: expected admission fault %s was not reported\n' "$admission_fault" >&2
        cat "$fixture_dir/$admission_fault-ci.log" >&2
        exit 1
    fi
done

for incremental_fault in global-disabled ordinary-override quoted-hash-double quoted-hash-single quoted-hash-multiline; do
    case "$incremental_fault" in
        global-disabled)
            expression='s/^  CARGO_INCREMENTAL: "0"$/  CARGO_INCREMENTAL: "1"/m' ;;
        ordinary-override)
            expression='s/(  native:.*?    timeout-minutes: 15\n)/$1    env:\n      CARGO_INCREMENTAL: "1"\n/s' ;;
        quoted-hash-double)
            expression='s/(      - name: Test repository automation\n        run: \|\n)/$1          printf " # quoted marker"; export CARGO_INCREMENTAL=0\n/s' ;;
        quoted-hash-single)
            expression='s/(      - name: Test repository automation\n        run: \|\n)/$1          printf \047 # quoted marker\047; export CARGO_INCREMENTAL=0\n/s' ;;
        quoted-hash-multiline)
            expression='s/(      - name: Test repository automation\n        run: \|\n)/$1          printf \047\n           # quoted marker\047; export CARGO_INCREMENTAL=0\n/s' ;;
    esac
    perl -0pe "$expression" "$fixture_dir/ci.yml" > "$fixture_dir/$incremental_fault-ci.yml"
    if ALPINE_CI_WORKFLOW="$fixture_dir/$incremental_fault-ci.yml" run_policy > "$fixture_dir/$incremental_fault.log" 2>&1; then
        printf 'policy test error: compilation mode fault %s unexpectedly passed\n' "$incremental_fault" >&2
        exit 1
    fi
    grep -Fq 'CI compilation mode must stay globally disabled and unscoped' "$fixture_dir/$incremental_fault.log"
done

perl -0pe 's/(  native-mutation:.*?    env:\n)/$1      # CARGO_INCREMENTAL controls compiler reuse only.\n/s' \
    "$fixture_dir/ci.yml" > "$fixture_dir/incremental-comment-ci.yml"
ALPINE_CI_WORKFLOW="$fixture_dir/incremental-comment-ci.yml" run_policy > "$fixture_dir/incremental-comment.log" 2>&1
sed 's/CARGO_TERM_COLOR: always/CARGO_TERM_COLOR: always # CARGO_INCREMENTAL stays scoped/' \
    "$fixture_dir/ci.yml" > "$fixture_dir/incremental-inline-comment-ci.yml"
ALPINE_CI_WORKFLOW="$fixture_dir/incremental-inline-comment-ci.yml" run_policy > "$fixture_dir/incremental-inline-comment.log" 2>&1
sed 's/CARGO_INCREMENTAL: "1"/CARGO_INCREMENTAL: "1" # native mutation copies only/' \
    "$fixture_dir/ci.yml" > "$fixture_dir/incremental-key-comment-ci.yml"
ALPINE_CI_WORKFLOW="$fixture_dir/incremental-key-comment-ci.yml" run_policy > "$fixture_dir/incremental-key-comment.log" 2>&1


for dispatch_fault in missing-base optional-base non-string-base classify-base; do
    case "$dispatch_fault" in
        missing-base)
            expression='s/    inputs:\n      base_sha:\n        description: [^\n]*\n        required: true\n        type: string\n//'
            diagnostic='CI dispatch must require an explicit string base_sha input'
            ;;
        optional-base)
            expression='s/(  workflow_dispatch:.*?required:) true/$1 false/s'
            diagnostic='CI dispatch must require an explicit string base_sha input'
            ;;
        non-string-base)
            expression='s/(  workflow_dispatch:.*?type:) string/$1 boolean/s'
            diagnostic='CI dispatch must require an explicit string base_sha input'
            ;;
        classify-base)
            expression='s/(  classify:.*?ALPINE_BASE_SHA: [^\n]*?) \|\| inputs\.base_sha/$1/s'
            diagnostic='CI classifier must bind the PR, push, or explicit dispatch base'
            ;;
    esac
    perl -0pe "$expression" "$fixture_dir/ci.yml" > "$fixture_dir/$dispatch_fault-ci.yml"
    if ALPINE_CI_WORKFLOW="$fixture_dir/$dispatch_fault-ci.yml" run_policy > "$fixture_dir/$dispatch_fault-ci.log" 2>&1; then
        printf 'policy test error: dispatch fault %s unexpectedly passed\n' "$dispatch_fault" >&2
        exit 1
    fi
    if ! grep -Fq "$diagnostic" "$fixture_dir/$dispatch_fault-ci.log"; then
        printf 'policy test error: expected dispatch fault %s was not reported\n' "$dispatch_fault" >&2
        cat "$fixture_dir/$dispatch_fault-ci.log" >&2
        exit 1
    fi
done

perl -0pe 's/(  preflight:.*?)(        run: scripts\/check-policy\.sh)/$1        run: true/s' \
    "$fixture_dir/ci.yml" > "$fixture_dir/bypassed-preflight-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/bypassed-preflight-ci.yml" run_policy > "$fixture_dir/bypassed-preflight-ci.log" 2>&1; then
    printf 'policy test error: bypassed fast policy preflight unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'CI preflight must validate technical repository policy before fan-out' \
    "$fixture_dir/bypassed-preflight-ci.log"; then
    printf 'policy test error: expected fast policy preflight failure was not reported\n' >&2
    cat "$fixture_dir/bypassed-preflight-ci.log" >&2
    exit 1
fi

perl -0pe 's/(  native:.*?needs:) \[classify, preflight\]/$1 classify/s' \
    "$fixture_dir/ci.yml" > "$fixture_dir/native-bypasses-preflight-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/native-bypasses-preflight-ci.yml" run_policy > "$fixture_dir/native-bypasses-preflight-ci.log" 2>&1; then
    printf 'policy test error: expensive CI job bypassing preflight unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'CI job native must wait for the fast policy preflight' \
    "$fixture_dir/native-bypasses-preflight-ci.log"; then
    printf 'policy test error: expected expensive preflight dependency failure was not reported\n' >&2
    cat "$fixture_dir/native-bypasses-preflight-ci.log" >&2
    exit 1
fi

sed '/PREFLIGHT_RESULT:.*needs.preflight.result/d' \
    "$fixture_dir/ci.yml" > "$fixture_dir/aggregate-omits-preflight-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/aggregate-omits-preflight-ci.yml" run_policy > "$fixture_dir/aggregate-omits-preflight-ci.log" 2>&1; then
    printf 'policy test error: aggregate omitting preflight unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'ci-pass must require the exact-head policy preflight' \
    "$fixture_dir/aggregate-omits-preflight-ci.log"; then
    printf 'policy test error: expected aggregate preflight failure was not reported\n' >&2
    cat "$fixture_dir/aggregate-omits-preflight-ci.log" >&2
    exit 1
fi

sed 's/if: ${{ always() && !cancelled() }}/if: always()/' \
    "$fixture_dir/ci.yml" > "$fixture_dir/canceled-aggregate-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/canceled-aggregate-ci.yml" run_policy > "$fixture_dir/canceled-aggregate-ci.log" 2>&1; then
    printf 'policy test error: aggregate admitted during workflow cancellation unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'ci-pass must run after ordinary failures but skip a canceled workflow' "$fixture_dir/canceled-aggregate-ci.log"; then
    printf 'policy test error: expected canceled aggregate admission failure was not reported\n' >&2
    cat "$fixture_dir/canceled-aggregate-ci.log" >&2
    exit 1
fi

sed 's/if: ${{ always() && !cancelled() }}/if: ${{ !cancelled() }}/' \
    "$fixture_dir/ci.yml" > "$fixture_dir/failed-dependency-skips-aggregate-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/failed-dependency-skips-aggregate-ci.yml" run_policy > "$fixture_dir/failed-dependency-skips-aggregate-ci.log" 2>&1; then
    printf 'policy test error: aggregate without ordinary-failure admission unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'ci-pass must run after ordinary failures but skip a canceled workflow' "$fixture_dir/failed-dependency-skips-aggregate-ci.log"; then
    printf 'policy test error: expected ordinary-failure aggregate admission failure was not reported\n' >&2
    cat "$fixture_dir/failed-dependency-skips-aggregate-ci.log" >&2
    exit 1
fi

sed 's/test "$2" = success || {/test "$2" != failure || {/' \
    "$fixture_dir/ci.yml" > "$fixture_dir/canceled-dependency-accepted-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/canceled-dependency-accepted-ci.yml" run_policy > "$fixture_dir/canceled-dependency-accepted-ci.log" 2>&1; then
    printf 'policy test error: aggregate accepting a canceled required dependency unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'ci-pass must reject every required result other than success' "$fixture_dir/canceled-dependency-accepted-ci.log"; then
    printf 'policy test error: expected canceled required dependency failure was not reported\n' >&2
    cat "$fixture_dir/canceled-dependency-accepted-ci.log" >&2
    exit 1
fi

sed "s/steps.upload-metal-shader-primary.outcome == 'failure'/steps.upload-metal-shader-primary.outcome == 'success'/" \
    "$fixture_dir/ci.yml" > "$fixture_dir/wrong-artifact-retry-route-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/wrong-artifact-retry-route-ci.yml" run_policy > "$fixture_dir/wrong-artifact-retry-route-ci.log" 2>&1; then
    printf 'policy test error: incorrectly routed Metal artifact retry unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'required Metal artifact upload must retain one identical blocking retry' "$fixture_dir/wrong-artifact-retry-route-ci.log"; then
    printf 'policy test error: expected Metal artifact retry routing failure was not reported\n' >&2
    cat "$fixture_dir/wrong-artifact-retry-route-ci.log" >&2
    exit 1
fi

sed "/steps.upload-metal-shader-primary.outcome == 'failure'/a\\
        continue-on-error: true" "$fixture_dir/ci.yml" > "$fixture_dir/nonblocking-artifact-retry-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/nonblocking-artifact-retry-ci.yml" run_policy > "$fixture_dir/nonblocking-artifact-retry-ci.log" 2>&1; then
    printf 'policy test error: nonblocking Metal artifact retry unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Eq 'continue-on-error is restricted|required Metal artifact upload must retain one identical blocking retry' "$fixture_dir/nonblocking-artifact-retry-ci.log"; then
    printf 'policy test error: expected blocking Metal artifact retry failure was not reported\n' >&2
    cat "$fixture_dir/nonblocking-artifact-retry-ci.log" >&2
    exit 1
fi


# Adversarial admission controls: declared steps and dependencies are not proof
# of execution if a step is skipped or a failed prerequisite is overridden.
for fault in fast-skip fast-ignore; do
    workflow="$fixture_dir/$fault.yml"
    case "$fault" in
        fast-skip) sed '/name: Require cheap source feedback before assurance fan-out/a\
        if: false
' .github/workflows/ci.yml > "$workflow" ;;
        fast-ignore) sed '/name: Require cheap source feedback before assurance fan-out/a\
        continue-on-error: true
' .github/workflows/ci.yml > "$workflow" ;;
    esac
    if ALPINE_CI_WORKFLOW="$workflow" scripts/check-policy.sh > "$fixture_dir/$fault.log" 2>&1; then
        printf 'policy test error: %s was accepted\n' "$fault" >&2
        exit 1
    fi
done
printf 'CI admission bypass policy controls passed\n'

for fault in aggregate-domain; do
    workflow="$fixture_dir/$fault.yml"
    case "$fault" in
        aggregate-domain) sed '/true|false) ;;/d' .github/workflows/ci.yml > "$workflow" ;;
    esac
    if ALPINE_CI_WORKFLOW="$workflow" scripts/check-policy.sh > "$fixture_dir/$fault.log" 2>&1; then
        printf 'policy test error: %s was accepted\n' "$fault" >&2; exit 1
    fi
done
printf 'CI aggregate requirement policy controls passed\n'

for fault in aggregate-skip aggregate-ignore; do
    workflow="$fixture_dir/$fault.yml"
    if [ "$fault" = aggregate-skip ]; then
        sed '/name: Require selected evidence/a\
        if: false
' .github/workflows/ci.yml > "$workflow"
    else
        sed '/name: Require selected evidence/a\
        continue-on-error: true
' .github/workflows/ci.yml > "$workflow"
    fi
    if ALPINE_CI_WORKFLOW="$workflow" scripts/check-policy.sh > "$fixture_dir/$fault.log" 2>&1; then
        printf 'policy test error: %s was accepted\n' "$fault" >&2; exit 1
    fi
done
printf 'CI aggregate execution policy controls passed\n'
