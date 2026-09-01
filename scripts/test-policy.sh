#!/bin/sh
set -eu

fixture_dir=$(mktemp -d)
trap 'rm -rf "$fixture_dir"' EXIT HUP INT TERM

cat > "$fixture_dir/mixed-assurance-failures.tsv" <<'EOF'
native-macos-arm64	Test workspace
ci-pass	Require selected evidence
metal-validation	Validate Metal
EOF
cat > "$fixture_dir/mixed-assurance-expected.tsv" <<'EOF'
native-macos-arm64	Test workspace
metal-validation	Validate Metal
EOF
scripts/filter-assurance-failures.sh < "$fixture_dir/mixed-assurance-failures.tsv" \
    > "$fixture_dir/mixed-assurance-actual.tsv"
cmp "$fixture_dir/mixed-assurance-expected.tsv" "$fixture_dir/mixed-assurance-actual.tsv"

printf 'ci-pass\tRequire selected evidence\n' > "$fixture_dir/aggregate-only.tsv"
scripts/filter-assurance-failures.sh < "$fixture_dir/aggregate-only.tsv" \
    > "$fixture_dir/aggregate-only-actual.tsv"
cmp "$fixture_dir/aggregate-only.tsv" "$fixture_dir/aggregate-only-actual.tsv"

: > "$fixture_dir/no-failures.tsv"
scripts/filter-assurance-failures.sh < "$fixture_dir/no-failures.tsv" \
    > "$fixture_dir/no-failures-actual.tsv"
cmp "$fixture_dir/no-failures.tsv" "$fixture_dir/no-failures-actual.tsv"

cat > "$fixture_dir/gh" <<'EOF'
#!/bin/sh
set -eu

if [ "$1" = api ]; then
    case "$2" in
        */issues/100/parent) printf '90\n' ;;
        */issues/90/parent) printf '80\n' ;;
        *) exit 1 ;;
    esac
    exit 0
fi

number=$3
field=
previous=
for argument in "$@"; do
    if [ "$previous" = --json ]; then
        field=$argument
        break
    fi
    previous=$argument
done

case "$field:$number" in
    labels:100) printf 'kind:task\n' ;;
    state:100) printf 'OPEN\n' ;;
    body:100) printf '### Parent capability or requirement\n\n#90\n' ;;
    labels:90)
        printf 'kind:requirement\n'
        if [ "${ALPINE_POLICY_FIXTURE:-valid}" != unapproved ]; then
            printf 'owner:approved\n'
        fi
        ;;
    state:90)
        if [ "${ALPINE_POLICY_FIXTURE:-valid}" = closed-requirement ]; then
            printf 'CLOSED\n'
        else
            printf 'OPEN\n'
        fi
        ;;
    body:90) printf '### Parent capability or requirement\n\n#80\n' ;;
    labels:80) printf 'kind:capability\nowner:approved\n' ;;
    state:80) printf 'OPEN\n' ;;
    labels,state,stateReason:70)
        if [ "${ALPINE_POLICY_FIXTURE:-valid}" = rejected-decision ]; then
            printf 'CLOSED\tNOT_PLANNED\tkind:decision\n'
        else
            printf 'CLOSED\tCOMPLETED\tkind:decision\n'
        fi
        ;;
    *) exit 1 ;;
esac
EOF
chmod +x "$fixture_dir/gh"

pr_body='## Closing issue

Closes #100

## Parent capability

#80

## Claims and evidence

AEP-0009-C05 and EV-0009-INTEGRATION05.

## Decision or research

#70

## Acceptance evidence

Policy fixture.

## Risk and scope

Policy fixture.

## Test plan

Policy fixture.

## Performance and memory

None.

## Release impact

release:feature

## Dependencies, provenance, and unsafe code

None.

## Adversarial review

Policy fixture.'

run_policy() {
    PATH="$fixture_dir:$PATH" \
    GH_REPOSITORY=dbuddha/alpine-gpui \
    ALPINE_BASE_SHA=fixture-base \
    ALPINE_HEAD_SHA=fixture-head \
    ALPINE_CHANGED_FILES="${ALPINE_POLICY_CHANGED_FILES:-crates/alpine-core/src/lib.rs}" \
    ALPINE_PR_BODY="$pr_body" \
    ALPINE_PR_LABELS=release:feature \
    ALPINE_PR_TITLE='feat(core): exercise approval fixture' \
    ALPINE_TLA_DRIVER="${ALPINE_TLA_DRIVER:-scripts/check-tla.sh}" \
    scripts/check-policy.sh
}

run_policy >/dev/null

cp scripts/check-tla.sh "$fixture_dir/check-tla.sh"
ALPINE_TLA_DRIVER="$fixture_dir/check-tla.sh" run_policy >/dev/null
sed 's/nightly) config=Nightly.cfg; lncheck=final ;;/nightly) config=Nightly.cfg; lncheck=default ;;/' \
    "$fixture_dir/check-tla.sh" > "$fixture_dir/periodic-nightly-tla.sh"
if ALPINE_TLA_DRIVER="$fixture_dir/periodic-nightly-tla.sh" run_policy > "$fixture_dir/periodic-nightly-tla.log" 2>&1; then
    printf 'policy test error: periodic Nightly liveness checking unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'TLA+ must preserve default pull-request checks and final-graph Nightly liveness checks' "$fixture_dir/periodic-nightly-tla.log"; then
    printf 'policy test error: expected final-graph Nightly liveness failure was not reported\n' >&2
    cat "$fixture_dir/periodic-nightly-tla.log" >&2
    exit 1
fi
unset ALPINE_TLA_DRIVER

cp .github/workflows/ci.yml "$fixture_dir/ci.yml"
ALPINE_CI_WORKFLOW="$fixture_dir/ci.yml" run_policy >/dev/null

cp .github/workflows/assurance-failure.yml "$fixture_dir/assurance-failure.yml"
sed '/scripts\/filter-assurance-failures.sh |/d' \
    "$fixture_dir/assurance-failure.yml" > "$fixture_dir/unfiltered-assurance-failure.yml"
if ALPINE_ASSURANCE_FAILURE_WORKFLOW="$fixture_dir/unfiltered-assurance-failure.yml" \
    run_policy > "$fixture_dir/unfiltered-assurance-failure.log" 2>&1; then
    printf 'policy test error: derivative aggregate failure routing unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'assurance routing must suppress derivative ci-pass failures through the tested selector' \
    "$fixture_dir/unfiltered-assurance-failure.log"; then
    printf 'policy test error: expected derivative aggregate routing failure was not reported\n' >&2
    cat "$fixture_dir/unfiltered-assurance-failure.log" >&2
    exit 1
fi
unset ALPINE_ASSURANCE_FAILURE_WORKFLOW

cp .github/workflows/nightly-assurance.yml "$fixture_dir/nightly-assurance.yml"
cp .github/actions/upload-required-artifact/action.yml "$fixture_dir/upload-required-artifact.yml"
ALPINE_NIGHTLY_ASSURANCE_WORKFLOW="$fixture_dir/nightly-assurance.yml" \
    ALPINE_REQUIRED_ARTIFACT_ACTION="$fixture_dir/upload-required-artifact.yml" \
    run_policy >/dev/null

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
    "$fixture_dir/nightly-assurance.yml" > "$fixture_dir/bypassed-required-artifact-nightly.yml"
if ALPINE_NIGHTLY_ASSURANCE_WORKFLOW="$fixture_dir/bypassed-required-artifact-nightly.yml" \
    run_policy > "$fixture_dir/bypassed-required-artifact-nightly.log" 2>&1; then
    printf 'policy test error: bypassed Nightly required artifact helper unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'Nightly required artifacts must use eight governed retries and retain one direct supplementary upload' \
    "$fixture_dir/bypassed-required-artifact-nightly.log"; then
    printf 'policy test error: expected Nightly required artifact bypass failure was not reported\n' >&2
    cat "$fixture_dir/bypassed-required-artifact-nightly.log" >&2
    exit 1
fi

perl -0pe 's/if-no-files-found: error/if-no-files-found: warn/' \
    "$fixture_dir/nightly-assurance.yml" > "$fixture_dir/downgraded-required-artifact-nightly.yml"
if ALPINE_NIGHTLY_ASSURANCE_WORKFLOW="$fixture_dir/downgraded-required-artifact-nightly.yml" \
    run_policy > "$fixture_dir/downgraded-required-artifact-nightly.log" 2>&1; then
    printf 'policy test error: downgraded Nightly required artifact unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'Nightly required artifacts must use eight governed retries and retain one direct supplementary upload' \
    "$fixture_dir/downgraded-required-artifact-nightly.log"; then
    printf 'policy test error: expected Nightly required artifact downgrade failure was not reported\n' >&2
    cat "$fixture_dir/downgraded-required-artifact-nightly.log" >&2
    exit 1
fi

perl -0pe 's/(  native-platform-contract-mutation:.*?)( --shard "\$\{\{ matrix\.shard \}\}")/$1/s' \
    "$fixture_dir/nightly-assurance.yml" > "$fixture_dir/unsharded-native-platform-nightly.yml"
if ALPINE_NIGHTLY_ASSURANCE_WORKFLOW="$fixture_dir/unsharded-native-platform-nightly.yml" \
    run_policy > "$fixture_dir/unsharded-native-platform-nightly.log" 2>&1; then
    printf 'policy test error: unsharded native platform Nightly unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'nightly assurance must shard native platform contracts and route Studio-only wrappers through Studio tests' \
    "$fixture_dir/unsharded-native-platform-nightly.log"; then
    printf 'policy test error: expected native platform Nightly sharding failure was not reported\n' >&2
    cat "$fixture_dir/unsharded-native-platform-nightly.log" >&2
    exit 1
fi
unset ALPINE_NIGHTLY_ASSURANCE_WORKFLOW

sed '/Hosted AppKit cannot qualify user-facing `performClose`/d' \
    "$fixture_dir/nightly-assurance.yml" > "$fixture_dir/unclassified-user-close-nightly.yml"
if ALPINE_NIGHTLY_ASSURANCE_WORKFLOW="$fixture_dir/unclassified-user-close-nightly.yml" \
    run_policy > "$fixture_dir/unclassified-user-close-nightly.log" 2>&1; then
    printf 'policy test error: unclassified physical user-close mutation scope unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'nightly assurance must shard native platform contracts and route Studio-only wrappers through Studio tests' \
    "$fixture_dir/unclassified-user-close-nightly.log"; then
    printf 'policy test error: expected physical user-close classification failure was not reported\n' >&2
    cat "$fixture_dir/unclassified-user-close-nightly.log" >&2
    exit 1
fi
unset ALPINE_NIGHTLY_ASSURANCE_WORKFLOW

sed 's/types: \[synchronize,/types: [opened, synchronize,/' \
    "$fixture_dir/ci.yml" > "$fixture_dir/opened-pr-fanout-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/opened-pr-fanout-ci.yml" run_policy > "$fixture_dir/opened-pr-fanout-ci.log" 2>&1; then
    printf 'policy test error: opened PR fan-out unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'CI pull_request triggers must start after label settlement and retain source and metadata events' \
    "$fixture_dir/opened-pr-fanout-ci.log"; then
    printf 'policy test error: expected settled PR trigger failure was not reported\n' >&2
    cat "$fixture_dir/opened-pr-fanout-ci.log" >&2
    exit 1
fi

sed "s/github.run_id || 'source'/github.event.action || 'source'/" \
    "$fixture_dir/ci.yml" > "$fixture_dir/shared-metadata-concurrency-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/shared-metadata-concurrency-ci.yml" run_policy > "$fixture_dir/shared-metadata-concurrency-ci.log" 2>&1; then
    printf 'policy test error: shared metadata concurrency unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'CI metadata events must not share a cancelable required-check concurrency group' \
    "$fixture_dir/shared-metadata-concurrency-ci.log"; then
    printf 'policy test error: expected metadata concurrency failure was not reported\n' >&2
    cat "$fixture_dir/shared-metadata-concurrency-ci.log" >&2
    exit 1
fi

sed 's/^  cancel-in-progress: .*/  cancel-in-progress: true/' \
    "$fixture_dir/ci.yml" > "$fixture_dir/unconditional-pr-cancellation-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/unconditional-pr-cancellation-ci.yml" run_policy > "$fixture_dir/unconditional-pr-cancellation-ci.log" 2>&1; then
    printf 'policy test error: unconditional PR cancellation unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'CI cancellation must be limited to source-changing or non-PR runs' \
    "$fixture_dir/unconditional-pr-cancellation-ci.log"; then
    printf 'policy test error: expected conditional cancellation failure was not reported\n' >&2
    cat "$fixture_dir/unconditional-pr-cancellation-ci.log" >&2
    exit 1
fi

perl -0pe 's/(  preflight:.*?)(        run: scripts\/check-policy\.sh)/$1        run: true/s' \
    "$fixture_dir/ci.yml" > "$fixture_dir/bypassed-preflight-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/bypassed-preflight-ci.yml" run_policy > "$fixture_dir/bypassed-preflight-ci.log" 2>&1; then
    printf 'policy test error: bypassed fast policy preflight unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'CI preflight must validate current repository and pull request policy before fan-out' \
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

perl -0pe 's/(  native-mutation:.*?)( --shard "\$\{\{ matrix\.shard \}\}")/$1/s' \
    "$fixture_dir/ci.yml" > "$fixture_dir/unsharded-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/unsharded-ci.yml" run_policy > "$fixture_dir/unsharded-ci.log" 2>&1; then
    printf 'policy test error: unsharded native mutation unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'pull-request native mutation must preserve all eleven scopes across sixteen deterministic shards' "$fixture_dir/unsharded-ci.log"; then
    printf 'policy test error: expected native-mutation sharding failure was not reported\n' >&2
    cat "$fixture_dir/unsharded-ci.log" >&2
    exit 1
fi

sed 's#--file tools/alpine-ax-client/src/native_factory.rs#--file tools/alpine-ax-client/src/missing-native-factory.rs#' \
    "$fixture_dir/ci.yml" > "$fixture_dir/missing-native-ax-factory-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/missing-native-ax-factory-ci.yml" run_policy > "$fixture_dir/missing-native-ax-factory-ci.log" 2>&1; then
    printf 'policy test error: missing native AX factory mutation unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'AX target-only mutation must leave Linux explicitly and retain one conditional Apple factory owner' "$fixture_dir/missing-native-ax-factory-ci.log"; then
    printf 'policy test error: expected missing native AX factory owner failure was not reported\n' >&2
    cat "$fixture_dir/missing-native-ax-factory-ci.log" >&2
    exit 1
fi

for target_only_file in native.rs native_factory.rs; do
    sed "s/ --exclude 'tools\/alpine-ax-client\/src\/${target_only_file}'//" \
        "$fixture_dir/ci.yml" > "$fixture_dir/linux-owned-ax-${target_only_file}.yml"
    if ALPINE_CI_WORKFLOW="$fixture_dir/linux-owned-ax-${target_only_file}.yml" run_policy > "$fixture_dir/linux-owned-ax-${target_only_file}.log" 2>&1; then
        printf 'policy test error: Linux-owned target-only AX mutation unexpectedly passed for %s\n' "$target_only_file" >&2
        exit 1
    fi
    if ! grep -Fq 'AX target-only mutation must leave Linux explicitly and retain one conditional Apple factory owner' "$fixture_dir/linux-owned-ax-${target_only_file}.log"; then
        printf 'policy test error: expected target-only AX ownership failure was not reported for %s\n' "$target_only_file" >&2
        cat "$fixture_dir/linux-owned-ax-${target_only_file}.log" >&2
        exit 1
    fi
done

perl -0pe 's/(  mutation-diff:.*?)( --shard "\$\{\{ matrix\.shard \}\}")/$1/s' \
    "$fixture_dir/ci.yml" > "$fixture_dir/unsharded-mutation-diff-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/unsharded-mutation-diff-ci.yml" run_policy > "$fixture_dir/unsharded-mutation-diff-ci.log" 2>&1; then
    printf 'policy test error: unsharded changed-code mutation unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'changed-code mutation must preserve shipping and assurance scopes across eight deterministic exact-head shards' "$fixture_dir/unsharded-mutation-diff-ci.log"; then
    printf 'policy test error: expected changed-code mutation sharding failure was not reported\n' >&2
    cat "$fixture_dir/unsharded-mutation-diff-ci.log" >&2
    exit 1
fi

sed "s/ --exclude 'apps\/alpine-studio\/src\/native_validation\/accessibility_process.rs'//" \
    "$fixture_dir/ci.yml" > "$fixture_dir/linux-owned-studio-process-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/linux-owned-studio-process-ci.yml" run_policy > "$fixture_dir/linux-owned-studio-process-ci.log" 2>&1; then
    printf 'policy test error: Linux-owned Studio process mutation unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'Studio accessibility process mutation must transfer explicitly from Linux to accessibility-scoped retained native shards' "$fixture_dir/linux-owned-studio-process-ci.log"; then
    printf 'policy test error: expected Studio process mutation ownership failure was not reported\n' >&2
    cat "$fixture_dir/linux-owned-studio-process-ci.log" >&2
    exit 1
fi

sed 's#--file apps/alpine-studio/src/native_validation/accessibility_process.rs#--file apps/alpine-studio/src/native_validation/missing-process.rs#' \
    "$fixture_dir/ci.yml" > "$fixture_dir/missing-native-studio-process-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/missing-native-studio-process-ci.yml" run_policy > "$fixture_dir/missing-native-studio-process-ci.log" 2>&1; then
    printf 'policy test error: missing native Studio process mutation unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'Studio accessibility process mutation must transfer explicitly from Linux to accessibility-scoped retained native shards' "$fixture_dir/missing-native-studio-process-ci.log"; then
    printf 'policy test error: expected missing native Studio process mutation failure was not reported\n' >&2
    cat "$fixture_dir/missing-native-studio-process-ci.log" >&2
    exit 1
fi

sed 's/ ALPINE_STUDIO_NATIVE_PROCESS_SCOPE=accessibility//' \
    "$fixture_dir/ci.yml" > "$fixture_dir/unscoped-native-studio-process-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/unscoped-native-studio-process-ci.yml" run_policy > "$fixture_dir/unscoped-native-studio-process-ci.log" 2>&1; then
    printf 'policy test error: unscoped native Studio process mutation unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'Studio accessibility process mutation must transfer explicitly from Linux to accessibility-scoped retained native shards' "$fixture_dir/unscoped-native-studio-process-ci.log"; then
    printf 'policy test error: expected unscoped native Studio process mutation failure was not reported\n' >&2
    cat "$fixture_dir/unscoped-native-studio-process-ci.log" >&2
    exit 1
fi

sed 's/|reset_native_validation_language_evidence//g' \
    "$fixture_dir/ci.yml" > "$fixture_dir/missing-language-evidence-owner-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/missing-language-evidence-owner-ci.yml" run_policy > "$fixture_dir/missing-language-evidence-owner-ci.log" 2>&1; then
    printf 'policy test error: unowned validation-only language evidence mutation unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'validation-only Studio language evidence mutation must transfer explicitly from Linux to retained Apple native shards' "$fixture_dir/missing-language-evidence-owner-ci.log"; then
    printf 'policy test error: expected validation-only language evidence mutation ownership failure was not reported\n' >&2
    cat "$fixture_dir/missing-language-evidence-owner-ci.log" >&2
    exit 1
fi

sed 's/, native-mutation]/]/' "$fixture_dir/ci.yml" > "$fixture_dir/unrequired-native-mutation-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/unrequired-native-mutation-ci.yml" run_policy > "$fixture_dir/unrequired-native-mutation-ci.log" 2>&1; then
    printf 'policy test error: unrequired native mutation unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'ci-pass must require and retain exact-head native mutation matrix evidence' "$fixture_dir/unrequired-native-mutation-ci.log"; then
    printf 'policy test error: expected native-mutation aggregation failure was not reported\n' >&2
    cat "$fixture_dir/unrequired-native-mutation-ci.log" >&2
    exit 1
fi
unset ALPINE_CI_WORKFLOW

ALPINE_POLICY_REFERENCE_INPUT='docs/research/index.md' run_policy >/dev/null

retired_roadmap='docs/ROAD''MAP.md'
if ALPINE_POLICY_REFERENCE_INPUT="$retired_roadmap" run_policy > "$fixture_dir/retired-reference.log" 2>&1; then
    printf 'policy test error: retired documentation reference unexpectedly passed\n' >&2
    exit 1
fi

if ! grep -Fq 'tracked files reference retired repository documents' "$fixture_dir/retired-reference.log"; then
    printf 'policy test error: expected retired-reference failure was not reported\n' >&2
    cat "$fixture_dir/retired-reference.log" >&2
    exit 1
fi
unset ALPINE_POLICY_REFERENCE_INPUT

if ALPINE_POLICY_FIXTURE=unapproved run_policy > "$fixture_dir/failure.log" 2>&1; then
    printf 'policy test error: unapproved requirement unexpectedly passed\n' >&2
    exit 1
fi

if ! grep -Fq 'requirement #90 requires owner:approved' "$fixture_dir/failure.log"; then
    printf 'policy test error: expected approval failure was not reported\n' >&2
    cat "$fixture_dir/failure.log" >&2
    exit 1
fi

if ALPINE_POLICY_FIXTURE=closed-requirement run_policy > "$fixture_dir/closed-requirement.log" 2>&1; then
    printf 'policy test error: closed requirement unexpectedly passed\n' >&2
    exit 1
fi

if ! grep -Fq 'requirement #90 must be open' "$fixture_dir/closed-requirement.log"; then
    printf 'policy test error: expected open-requirement failure was not reported\n' >&2
    cat "$fixture_dir/closed-requirement.log" >&2
    exit 1
fi

ALPINE_POLICY_CHANGED_FILES=ARCHITECTURE.md run_policy >/dev/null

if ALPINE_POLICY_CHANGED_FILES=ARCHITECTURE.md ALPINE_POLICY_FIXTURE=rejected-decision run_policy > "$fixture_dir/decision-failure.log" 2>&1; then
    printf 'policy test error: rejected decision unexpectedly passed\n' >&2
    exit 1
fi

if ! grep -Fq 'architecture changes require a closed kind:decision issue' "$fixture_dir/decision-failure.log"; then
    printf 'policy test error: expected accepted-decision failure was not reported\n' >&2
    cat "$fixture_dir/decision-failure.log" >&2
    exit 1
fi

native_surface_fixture="$(mktemp -d)"
cp .github/workflows/nightly-assurance.yml "${native_surface_fixture}/nightly-assurance.yml"
sed '/shard: "7\/8"/d' "${native_surface_fixture}/nightly-assurance.yml" \
  > "${native_surface_fixture}/nightly-assurance.modified.yml"
mv "${native_surface_fixture}/nightly-assurance.modified.yml" \
  "${native_surface_fixture}/nightly-assurance.yml"
if ALPINE_NIGHTLY_ASSURANCE_WORKFLOW="${native_surface_fixture}/nightly-assurance.yml" scripts/check-policy.sh >/dev/null 2>&1; then
  echo "policy test failure: missing native surface shard was accepted" >&2
  rm -rf "${native_surface_fixture}"
  exit 1
fi
rm -rf "${native_surface_fixture}"

native_studio_fixture="$(mktemp -d)"
sed '/^  native-studio-contract-mutation:/d' .github/workflows/nightly-assurance.yml \
  > "${native_studio_fixture}/nightly-assurance.yml"
if ALPINE_NIGHTLY_ASSURANCE_WORKFLOW="${native_studio_fixture}/nightly-assurance.yml" scripts/check-policy.sh >/dev/null 2>&1; then
  echo "policy test failure: missing Studio native contract job was accepted" >&2
  rm -rf "${native_studio_fixture}"
  exit 1
fi
rm -rf "${native_studio_fixture}"

native_runtime_filter_fixture="$(mktemp -d)"
sed '/--file crates\/alpine-runtime\/src\/lib.rs/s/ native_process$//' \
  .github/workflows/nightly-assurance.yml \
  > "${native_runtime_filter_fixture}/nightly-assurance.yml"
if ALPINE_NIGHTLY_ASSURANCE_WORKFLOW="${native_runtime_filter_fixture}/nightly-assurance.yml" scripts/check-policy.sh >/dev/null 2>&1; then
  echo "policy test failure: unfiltered Nightly runtime mutation control was accepted" >&2
  rm -rf "${native_runtime_filter_fixture}"
  exit 1
fi
rm -rf "${native_runtime_filter_fixture}"

ci_native_fixture="$(mktemp -d)"
sed 's#--file crates/alpine-platform-macos/src/native.rs#--file crates/alpine-platform-macos/src/native-missing.rs#' .github/workflows/ci.yml \
  > "${ci_native_fixture}/ci.yml"
if ALPINE_CI_WORKFLOW="${ci_native_fixture}/ci.yml" scripts/check-policy.sh >/dev/null 2>&1; then
  echo "policy test failure: missing exact-head native surface scope was accepted" >&2
  rm -rf "${ci_native_fixture}"
  exit 1
fi
rm -rf "${ci_native_fixture}"

ci_runtime_filter_fixture="$(mktemp -d)"
sed '/--file crates\/alpine-runtime\/src\/lib.rs/s/ native_process$//' \
  .github/workflows/ci.yml > "${ci_runtime_filter_fixture}/ci.yml"
if ALPINE_CI_WORKFLOW="${ci_runtime_filter_fixture}/ci.yml" scripts/check-policy.sh >/dev/null 2>&1; then
  echo "policy test failure: unfiltered exact-head runtime mutation control was accepted" >&2
  rm -rf "${ci_runtime_filter_fixture}"
  exit 1
fi
rm -rf "${ci_runtime_filter_fixture}"

ci_native_shard_fixture="$(mktemp -d)"
sed '/shard: 15\/16/d' .github/workflows/ci.yml \
  > "${ci_native_shard_fixture}/ci.yml"
if ALPINE_CI_WORKFLOW="${ci_native_shard_fixture}/ci.yml" scripts/check-policy.sh >/dev/null 2>&1; then
  echo "policy test failure: missing exact-head native mutation shard was accepted" >&2
  rm -rf "${ci_native_shard_fixture}"
  exit 1
fi
rm -rf "${ci_native_shard_fixture}"

printf 'repository policy tests passed\n'
