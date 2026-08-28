#!/bin/sh
set -eu

max_attempts=3
attempt=1

while [ "$attempt" -le "$max_attempts" ]; do
    printf 'Kani setup attempt %s/%s\n' "$attempt" "$max_attempts"
    if cargo kani setup; then
        exit 0
    fi

    if [ "$attempt" -eq "$max_attempts" ]; then
        break
    fi

    delay_seconds=$((attempt * 5))
    printf 'Kani setup transport failed; retrying in %s seconds\n' "$delay_seconds" >&2
    sleep "$delay_seconds"
    attempt=$((attempt + 1))
done

printf 'Kani setup failed after %s attempts\n' "$max_attempts" >&2
exit 1
