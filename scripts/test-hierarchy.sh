#!/bin/sh
set -eu

fixture_dir=$(mktemp -d)
trap 'rm -rf "$fixture_dir"' EXIT HUP INT TERM

cat > "$fixture_dir/gh" <<'EOF'
#!/bin/sh
set -eu

if [ "$1" = api ]; then
    endpoint=
    for argument in "$@"; do
        case "$argument" in
            repos/*) endpoint=$argument ;;
        esac
    done
    case "${ALPINE_HIERARCHY_FIXTURE:?}:$endpoint" in
        close-parent:*/issues/100/sub_issues?*) printf '[]\n' ;;
        close-parent:*/issues/100/parent) printf '90\n' ;;
        close-parent:*/issues/90/sub_issues?*) printf '[{"state":"closed"},{"state":"closed"}]\n' ;;
        reject-parent-close:*/issues/90/sub_issues?*) printf '[{"state":"closed"},{"state":"open"}]\n' ;;
        reopen-parent:*/issues/100/parent) printf '90\n' ;;
        *) exit 1 ;;
    esac
    exit 0
fi

command=$2
number=$3
if [ "$command" = view ]; then
    case "${ALPINE_HIERARCHY_FIXTURE:?}:$number:$*" in
        close-parent:90:*labels*) printf 'kind:requirement\nowner:approved\n' ;;
        reopen-parent:90:*state*) printf 'CLOSED\n' ;;
        *) exit 1 ;;
    esac
else
    printf '%s %s\n' "$command" "$number" >> "${ALPINE_HIERARCHY_LOG:?}"
fi
EOF
chmod +x "$fixture_dir/gh"

run_fixture() {
    fixture=$1
    issue=$2
    action=$3
    log="$fixture_dir/$fixture.log"
    : > "$log"
    PATH="$fixture_dir:$PATH" \
    GITHUB_REPOSITORY=dbuddha/alpine-gpui \
    ALPINE_ISSUE_NUMBER=$issue \
    ALPINE_ISSUE_ACTION=$action \
    ALPINE_HIERARCHY_FIXTURE=$fixture \
    ALPINE_HIERARCHY_LOG=$log \
    ALPINE_ENFORCE_EVIDENCE=false \
    scripts/reconcile-issue-hierarchy.sh
}

run_fixture close-parent 100 closed
grep -Fxq 'close 90' "$fixture_dir/close-parent.log"

run_fixture reject-parent-close 90 closed
grep -Fxq 'reopen 90' "$fixture_dir/reject-parent-close.log"

run_fixture reopen-parent 100 reopened
grep -Fxq 'reopen 90' "$fixture_dir/reopen-parent.log"

printf 'issue hierarchy tests passed\n'
