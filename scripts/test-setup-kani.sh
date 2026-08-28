#!/bin/sh
set -eu

fixture_dir=$(mktemp -d)
trap 'rm -rf "$fixture_dir"' EXIT HUP INT TERM
mkdir -p "$fixture_dir/bin"

cat > "$fixture_dir/bin/cargo" <<'EOF'
#!/bin/sh
set -eu

if [ "$#" -ne 2 ] || [ "$1" != kani ] || [ "$2" != setup ]; then
    printf 'unexpected cargo invocation: %s\n' "$*" >&2
    exit 64
fi

count=0
if [ -f "$ALPINE_TEST_ATTEMPT_FILE" ]; then
    count=$(cat "$ALPINE_TEST_ATTEMPT_FILE")
fi
count=$((count + 1))
printf '%s\n' "$count" > "$ALPINE_TEST_ATTEMPT_FILE"
[ "$count" -ge "$ALPINE_TEST_SUCCEED_ON" ]
EOF

cat > "$fixture_dir/bin/sleep" <<'EOF'
#!/bin/sh
set -eu
printf '%s\n' "$1" >> "$ALPINE_TEST_SLEEP_FILE"
EOF

chmod +x "$fixture_dir/bin/cargo" "$fixture_dir/bin/sleep"

assert_file() {
    file=$1
    expected=$2
    actual=$(cat "$file")
    if [ "$actual" != "$expected" ]; then
        printf 'Kani setup test error: expected %s, got %s\n' "$expected" "$actual" >&2
        exit 1
    fi
}

attempt_file="$fixture_dir/success-attempts"
sleep_file="$fixture_dir/success-sleeps"
PATH="$fixture_dir/bin:$PATH" \
ALPINE_TEST_ATTEMPT_FILE="$attempt_file" \
ALPINE_TEST_SLEEP_FILE="$sleep_file" \
ALPINE_TEST_SUCCEED_ON=3 \
    scripts/setup-kani.sh
assert_file "$attempt_file" 3
assert_file "$sleep_file" "5
10"

attempt_file="$fixture_dir/failure-attempts"
sleep_file="$fixture_dir/failure-sleeps"
if PATH="$fixture_dir/bin:$PATH" \
    ALPINE_TEST_ATTEMPT_FILE="$attempt_file" \
    ALPINE_TEST_SLEEP_FILE="$sleep_file" \
    ALPINE_TEST_SUCCEED_ON=4 \
        scripts/setup-kani.sh; then
    printf 'Kani setup test error: exhausted retries passed\n' >&2
    exit 1
fi
assert_file "$attempt_file" 3
assert_file "$sleep_file" "5
10"

printf 'Kani setup retry tests passed\n'
