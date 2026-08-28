#!/bin/sh
set -eu

fixture_dir=$(mktemp -d)
trap 'rm -rf "$fixture_dir"' EXIT HUP INT TERM

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

perl -0pe 's/(  native-mutation:.*?)( --shard "\$\{\{ matrix\.shard \}\}")/$1/s' \
    "$fixture_dir/ci.yml" > "$fixture_dir/unsharded-ci.yml"
if ALPINE_CI_WORKFLOW="$fixture_dir/unsharded-ci.yml" run_policy > "$fixture_dir/unsharded-ci.log" 2>&1; then
    printf 'policy test error: unsharded native mutation unexpectedly passed\n' >&2
    exit 1
fi
if ! grep -Fq 'pull-request native mutation must preserve all ten scopes across eight deterministic shards' "$fixture_dir/unsharded-ci.log"; then
    printf 'policy test error: expected native-mutation sharding failure was not reported\n' >&2
    cat "$fixture_dir/unsharded-ci.log" >&2
    exit 1
fi

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

printf 'repository policy tests passed\n'
