#!/bin/bash
set -euo pipefail

fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT HUP INT TERM
mkdir "$fixture/bin"

jq -n '
    def job($id; $name; $conclusion; $end): {
        id: $id, name: $name, status: "completed", conclusion: $conclusion,
        check_run_url: ("https://api.github.com/repos/fixture/repo/check-runs/" + ($id | tostring)),
        started_at: "2026-09-07T00:00:00Z", completed_at: $end,
        steps: [{name: "Check", conclusion: (if $conclusion == "failure" then "failure" else "cancelled" end)}]
    };
    {jobs: [
        job(1; "native"; "failure"; "2026-09-07T00:01:00Z"),
        job(2; "ci-pass"; "failure"; "2026-09-07T00:01:00Z"),
        job(3; "budget"; "cancelled"; "2026-09-07T00:30:00Z"),
        job(4; "manual"; "cancelled"; "2026-09-07T00:30:00Z"),
        job(5; "superseded"; "cancelled"; "2026-09-07T00:30:00Z"),
        job(6; "too-early"; "cancelled"; "2026-09-07T00:29:59Z"),
        job(7; "warning"; "cancelled"; "2026-09-07T00:30:00Z"),
        job(8; "source-message"; "cancelled"; "2026-09-07T00:30:00Z"),
        job(9; "prefixed-message"; "cancelled"; "2026-09-07T00:30:00Z"),
        job(10; "structured"; "timed_out"; "2026-09-07T00:30:00Z"),
        (job(11; "unfinished"; "failure"; "2026-09-07T00:30:00Z") | .status = "in_progress")
    ]}
' > "$fixture/jobs.json"
cat > "$fixture/timeout.json" <<'EOF'
[{"path":".github","annotation_level":"failure","message":"The job has exceeded the maximum execution time of 30m0s"}]
EOF
cat > "$fixture/bin/gh" <<'EOF'
#!/bin/bash
set -euo pipefail
[[ "$#" == 3 && "$1" == api && "$3" == --paginate ]] || exit 90
printf '%s\n' "$2" >> "$COLLECTOR_FIXTURE/calls"
if [[ "$2" == repos/fixture/repo/actions/runs/101/jobs ]]; then
    case "$COLLECTOR_CASE" in
        api-failure) exit 31 ;;
        malformed-jobs) printf '{}\n' ;;
        empty) printf '{"jobs":[]}\n' ;;
        pagination) jq '{jobs:.jobs[0:2]}, {jobs:.jobs[2:]}' "$COLLECTOR_FIXTURE/jobs.json" ;;
        foreign-url) jq '.jobs[2].check_run_url="https://example.invalid/check-runs/3"' "$COLLECTOR_FIXTURE/jobs.json" ;;
        malformed-time) jq '.jobs[2].started_at="invalid"' "$COLLECTOR_FIXTURE/jobs.json" ;;
        *) cat "$COLLECTOR_FIXTURE/jobs.json" ;;
    esac
    exit 0
fi
case "$2" in
    https://api.github.com/repos/fixture/repo/check-runs/3/annotations)
        case "$COLLECTOR_CASE" in
            annotation-failure) exit 32 ;;
            malformed-annotations) printf '{}\n'; exit 0 ;;
            pagination) printf '[]\n' ;;
        esac
        cat "$COLLECTOR_FIXTURE/timeout.json" ;;
    https://api.github.com/repos/fixture/repo/check-runs/4/annotations) printf '[]\n' ;;
    https://api.github.com/repos/fixture/repo/check-runs/5/annotations)
        printf '[{"path":".github","annotation_level":"failure","message":"The operation was canceled."}]\n' ;;
    https://api.github.com/repos/fixture/repo/check-runs/6/annotations) cat "$COLLECTOR_FIXTURE/timeout.json" ;;
    https://api.github.com/repos/fixture/repo/check-runs/7/annotations) jq '.[0].annotation_level="warning"' "$COLLECTOR_FIXTURE/timeout.json" ;;
    https://api.github.com/repos/fixture/repo/check-runs/8/annotations) jq '.[0].path="src/lib.rs"' "$COLLECTOR_FIXTURE/timeout.json" ;;
    https://api.github.com/repos/fixture/repo/check-runs/9/annotations) jq '.[0].message="prefix: The job has exceeded the maximum execution time of 30m0s"' "$COLLECTOR_FIXTURE/timeout.json" ;;
    *) exit 91 ;;
esac
EOF
chmod +x "$fixture/bin/gh"
export COLLECTOR_FIXTURE="$fixture"
export GITHUB_REPOSITORY=fixture/repo RUN_ID=101 GITHUB_API_URL=https://api.github.com
export PATH="$fixture/bin:$PATH"
printf 'native\tCheck\nci-pass\tCheck\nbudget\tjob timed out: Check\nstructured\tjob timed out: Check\n' > "$fixture/expected"
printf 'native\tCheck\nbudget\tjob timed out: Check\nstructured\tjob timed out: Check\n' > "$fixture/filtered-expected"

for scenario in mixed pagination; do
    COLLECTOR_CASE="$scenario" scripts/collect-assurance-failures.sh > "$fixture/actual"
    cmp "$fixture/expected" "$fixture/actual"
    scripts/filter-assurance-failures.sh < "$fixture/actual" > "$fixture/filtered"
    cmp "$fixture/filtered-expected" "$fixture/filtered"
done
COLLECTOR_CASE=empty scripts/collect-assurance-failures.sh > "$fixture/empty"
test ! -s "$fixture/empty"

for scenario in api-failure malformed-jobs annotation-failure malformed-annotations foreign-url malformed-time; do
    if COLLECTOR_CASE="$scenario" scripts/collect-assurance-failures.sh > "$fixture/rejected" 2> "$fixture/error"; then
        printf 'collector test: accepted %s\n' "$scenario" >&2
        exit 1
    fi
    test ! -s "$fixture/rejected"
    test -s "$fixture/error"
done
for identity in repo run; do
    : > "$fixture/calls"
    if (
        if [[ "$identity" == repo ]]; then export GITHUB_REPOSITORY=fixture/repo/extra;
        else export RUN_ID=../101; fi
        COLLECTOR_CASE=mixed scripts/collect-assurance-failures.sh
    ) > "$fixture/rejected" 2> "$fixture/error"; then
        printf 'collector test: accepted invalid %s identity\n' "$identity" >&2
        exit 1
    fi
    test ! -s "$fixture/rejected"
    test ! -s "$fixture/calls"
done

printf 'assurance failure collector controls passed\n'
