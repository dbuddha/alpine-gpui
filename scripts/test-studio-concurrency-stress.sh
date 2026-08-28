#!/bin/sh
set -eu

iterations=${ALPINE_STUDIO_CONCURRENCY_STRESS_ITERATIONS:-25}
case "$iterations" in
    '' | *[!0-9]*)
        echo "Studio concurrency stress iterations must be an integer" >&2
        exit 1
        ;;
esac
if [ "$iterations" -lt 1 ] || [ "$iterations" -gt 100 ]; then
    echo "Studio concurrency stress iterations must be between 1 and 100" >&2
    exit 1
fi

unset ALPINE_RUST_ANALYZER || true
export CARGO_TERM_COLOR=never

run_exact_test() {
    test_name=$1
    iteration=1
    while [ "$iteration" -le "$iterations" ]; do
        if ! output=$(cargo test --locked -p alpine-studio --lib "$test_name" -- --exact 2>&1); then
            printf '%s\n' "$output" >&2
            echo "Studio concurrency stress failed: $test_name iteration $iteration/$iterations" >&2
            return 1
        fi
        case "$output" in
            *"test result: ok. 1 passed; 0 failed;"*) ;;
            *)
                printf '%s\n' "$output" >&2
                echo "Studio concurrency stress did not execute exactly one test: $test_name" >&2
                return 1
                ;;
        esac
        iteration=$((iteration + 1))
    done
    echo "Studio concurrency stress passed $iterations iterations: $test_name"
}

run_exact_test 'tests::runtime_find_worker_admits_current_results_and_schedules_replacement'
run_exact_test 'project_search_tests::runtime_project_search_admits_workers_and_rolls_back_submission_failures'
run_exact_test 'tests::runtime_quick_open_worker_admits_inventory_and_ranked_results'
