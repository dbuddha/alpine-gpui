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
    scripts/check-policy.sh
}

run_policy >/dev/null

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
