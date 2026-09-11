#!/bin/bash
set -euo pipefail
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT HUP INT TERM
cat > "$fixture/gh" <<'EOF'
#!/bin/bash
set -euo pipefail
case "$1:$2" in
    api:*/actions/runs/123)
        if [[ "${FIXTURE_RACE:-false}" == true ]]; then
            count=$(cat "$FIXTURE_COUNTER" 2>/dev/null || echo 0)
            count=$((count + 1)); echo "$count" > "$FIXTURE_COUNTER"
            if [[ "$count" -ge 3 ]]; then FIXTURE_ATTEMPT=2; fi
        fi
        jq -n --arg conclusion "$FIXTURE_CONCLUSION" --arg status "$FIXTURE_STATUS" --arg branch "$FIXTURE_BRANCH" --arg head "$FIXTURE_HEAD" --argjson attempt "$FIXTURE_ATTEMPT" '{head_sha:$head,status:$status,head_branch:$branch,conclusion:$conclusion,name:"CI",html_url:"https://github.com/example/repo/actions/runs/123",run_attempt:$attempt}' ;;
    api:*/git/ref/heads/main)
        if [[ "$FIXTURE_MAIN" == error ]]; then exit 1; fi
        printf '%s\n' "$FIXTURE_MAIN" ;;
    api:*/actions/runs/123/jobs)
        echo '{"jobs":[{"name":"native-macos-arm64","status":"completed","conclusion":"failure","steps":[{"name":"Native tests","conclusion":"failure"}]}]}' ;;
    issue:list)
        [[ ! -f "$FIXTURE_LOG" ]] || printf '42\n' ;;
    issue:create|issue:comment)
        printf '%s\n' "$2" >> "$FIXTURE_LOG" ;;
    *) printf 'unexpected gh call %s\n' "$*" >&2; exit 1 ;;
esac
EOF
chmod +x "$fixture/gh"
export PATH="$fixture:$PATH" GITHUB_REPOSITORY=example/repo RUN_ID=123 RUN_ATTEMPT=1
export FIXTURE_HEAD=1111111111111111111111111111111111111111
export FIXTURE_MAIN=$FIXTURE_HEAD FIXTURE_STATUS=completed FIXTURE_BRANCH=main FIXTURE_ATTEMPT=1
export FIXTURE_LOG="$fixture/published"
for conclusion in failure timed_out; do
    export FIXTURE_CONCLUSION=$conclusion
    rm -f "$FIXTURE_LOG"
    scripts/route-assurance-failures.sh
    scripts/route-assurance-failures.sh
    printf 'create\ncomment\n' > "$fixture/expected"
    cmp "$fixture/expected" "$FIXTURE_LOG"
done
for conclusion in success skipped cancelled neutral; do
    export FIXTURE_CONCLUSION=$conclusion
    rm -f "$FIXTURE_LOG"
    scripts/route-assurance-failures.sh
    test ! -e "$FIXTURE_LOG"
done
export FIXTURE_CONCLUSION=failure
for state in stale in-progress other-branch superseded-attempt; do
    export FIXTURE_MAIN=$FIXTURE_HEAD FIXTURE_STATUS=completed FIXTURE_BRANCH=main FIXTURE_ATTEMPT=1
    case "$state" in
        stale) export FIXTURE_MAIN=2222222222222222222222222222222222222222 ;;
        in-progress) export FIXTURE_STATUS=in_progress ;;
        other-branch) export FIXTURE_BRANCH=feature ;;
        superseded-attempt) export FIXTURE_ATTEMPT=2 ;;
    esac
    scripts/route-assurance-failures.sh
    test ! -e "$FIXTURE_LOG"
done
export FIXTURE_MAIN=$FIXTURE_HEAD FIXTURE_STATUS=completed FIXTURE_BRANCH=main FIXTURE_ATTEMPT=1
export FIXTURE_RACE=true FIXTURE_COUNTER="$fixture/counter"
scripts/route-assurance-failures.sh
test ! -e "$FIXTURE_LOG"
unset FIXTURE_RACE
export FIXTURE_MAIN=error
if scripts/route-assurance-failures.sh > "$fixture/error" 2>&1; then
    echo 'failed main lookup was accepted' >&2; exit 1
fi
test ! -e "$FIXTURE_LOG"
echo 'current-main failure routing fixtures passed'
