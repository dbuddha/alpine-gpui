#!/bin/sh
set -eu

fixture_dir=$(mktemp -d)
trap 'rm -rf "$fixture_dir"' EXIT HUP INT TERM

dependencies=$(cat assurance/alpine-studio-dependencies.txt)

ALPINE_PRODUCT_DEPENDENCY_INPUT=$dependencies \
ALPINE_PRODUCT_SYMBOL_INPUT='_objc_msgSend' \
ALPINE_PRODUCT_STRING_INPUT='Alpine Studio' \
    scripts/check-product-boundary.sh --binary >/dev/null

invalid_dependencies=$(printf '%s\nreqwest v0.12.0\n' "$dependencies")
if ALPINE_PRODUCT_DEPENDENCY_INPUT=$invalid_dependencies \
    scripts/check-product-boundary.sh > "$fixture_dir/dependency.log" 2>&1; then
    printf 'product boundary test error: dependency widening unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'shipping dependency closure changed' "$fixture_dir/dependency.log"

if ALPINE_PRODUCT_DEPENDENCY_INPUT=$dependencies \
    ALPINE_PRODUCT_SOURCE_INPUT='use std::net::TcpStream;' \
    scripts/check-product-boundary.sh > "$fixture_dir/source.log" 2>&1; then
    printf 'product boundary test error: network source unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'shipping source contains a network capability' "$fixture_dir/source.log"

if ALPINE_PRODUCT_DEPENDENCY_INPUT=$dependencies \
    ALPINE_PRODUCT_SOURCE_INPUT='' \
    ALPINE_PRODUCT_FEATURE_INPUT='telemetry = []' \
    scripts/check-product-boundary.sh > "$fixture_dir/feature.log" 2>&1; then
    printf 'product boundary test error: excluded feature unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'shipping manifest declares an excluded product feature' "$fixture_dir/feature.log"

if ALPINE_PRODUCT_DEPENDENCY_INPUT=$dependencies \
    ALPINE_PRODUCT_SOURCE_INPUT='' \
    ALPINE_PRODUCT_FEATURE_INPUT='' \
    ALPINE_PRODUCT_PATH_INPUT='apps/alpine-studio/src/plugin_host.rs' \
    scripts/check-product-boundary.sh > "$fixture_dir/path.log" 2>&1; then
    printf 'product boundary test error: excluded subsystem path unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'shipping source path declares an excluded subsystem' "$fixture_dir/path.log"

if ALPINE_PRODUCT_DEPENDENCY_INPUT=$dependencies \
    ALPINE_PRODUCT_SOURCE_INPUT='' \
    ALPINE_PRODUCT_FEATURE_INPUT='plugins = []' \
    scripts/check-product-boundary.sh > "$fixture_dir/plural-feature.log" 2>&1; then
    printf 'product boundary test error: plural excluded feature unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'shipping manifest declares an excluded product feature' "$fixture_dir/plural-feature.log"

if ALPINE_PRODUCT_DEPENDENCY_INPUT=$dependencies \
    ALPINE_PRODUCT_SOURCE_INPUT='' \
    ALPINE_PRODUCT_FEATURE_INPUT='' \
    ALPINE_PRODUCT_PATH_INPUT='apps/alpine-studio/src/extensions/mod.rs' \
    scripts/check-product-boundary.sh > "$fixture_dir/plural-path.log" 2>&1; then
    printf 'product boundary test error: plural excluded subsystem path unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'shipping source path declares an excluded subsystem' "$fixture_dir/plural-path.log"

if ALPINE_PRODUCT_DEPENDENCY_INPUT=$dependencies \
    ALPINE_PRODUCT_SOURCE_INPUT='' \
    ALPINE_PRODUCT_FEATURE_INPUT='' \
    ALPINE_PRODUCT_PATH_INPUT='' \
    ALPINE_PRODUCT_SYMBOL_INPUT='_socket' \
    ALPINE_PRODUCT_STRING_INPUT='Alpine Studio' \
    scripts/check-product-boundary.sh --binary > "$fixture_dir/symbol.log" 2>&1; then
    printf 'product boundary test error: network symbol unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'release binary imports network symbols' "$fixture_dir/symbol.log"

if ALPINE_PRODUCT_DEPENDENCY_INPUT=$dependencies \
    ALPINE_PRODUCT_SOURCE_INPUT='' \
    ALPINE_PRODUCT_FEATURE_INPUT='' \
    ALPINE_PRODUCT_PATH_INPUT='' \
    ALPINE_PRODUCT_SYMBOL_INPUT='_objc_msgSend' \
    ALPINE_PRODUCT_STRING_INPUT='https://telemetry.invalid/v1' \
    scripts/check-product-boundary.sh --binary > "$fixture_dir/endpoint.log" 2>&1; then
    printf 'product boundary test error: endpoint string unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'release binary contains a network endpoint' "$fixture_dir/endpoint.log"

# Exercise the real workflow command using GitHub's shell semantics. The
# isolated fixture avoids overwriting evidence from an actual release build.
mkdir -p "$fixture_dir/workflow/scripts" "$fixture_dir/workflow/assurance" "$fixture_dir/workflow/target"
cp scripts/check-product-boundary.sh "$fixture_dir/workflow/scripts/"
cp assurance/alpine-studio-dependencies.txt "$fixture_dir/workflow/assurance/"
step=$(awk '/- name: Audit Alpine Studio release product boundary/ { active=1; next }
    active && /- name:/ { exit } active { print }' .github/workflows/ci.yml)
command=$(printf '%s\n' "$step" | sed -n 's/^        run: //p')
test -n "$command"
if printf '%s\n' "$step" | grep -Fq '        shell: bash'; then
    set -- --noprofile --norc -eo pipefail
else
    set -- -e
fi
for fixture in valid invalid; do
    input=$dependencies
    [ "$fixture" != invalid ] || input=$invalid_dependencies
    result=0
    (cd "$fixture_dir/workflow" &&
        ALPINE_PRODUCT_DEPENDENCY_INPUT=$input \
        ALPINE_PRODUCT_SOURCE_INPUT='' ALPINE_PRODUCT_FEATURE_INPUT='' \
        ALPINE_PRODUCT_PATH_INPUT='' ALPINE_PRODUCT_SYMBOL_INPUT='_objc_msgSend' \
        ALPINE_PRODUCT_STRING_INPUT='Alpine Studio' \
        bash "$@" -c "$command") > "$fixture_dir/workflow-$fixture.log" 2>&1 || result=$?
    expected=0
    [ "$fixture" != invalid ] || expected=1
    if [ "$result" -ne "$expected" ]; then
        printf 'product boundary workflow returned %s for %s input, expected %s\n' "$result" "$fixture" "$expected" >&2
        cat "$fixture_dir/workflow-$fixture.log" >&2
        exit 1
    fi
done

# Cargo's human-facing color setting must not affect the parsed closure, and
# an upstream failure must propagate even when Cargo printed a valid closure.
mkdir -p "$fixture_dir/cargo-bin"
cat > "$fixture_dir/cargo-bin/cargo" <<'CARGO'
#!/bin/sh
[ "$1 $2 $3" = 'tree --color never' ] || exit 73
cat "$ALPINE_BOUNDARY_TEST_CLOSURE"
exit "$ALPINE_BOUNDARY_TEST_EXIT"
CARGO
chmod +x "$fixture_dir/cargo-bin/cargo"
for status in 0 42; do
    result=0
    CARGO_TERM_COLOR=always PATH="$fixture_dir/cargo-bin:$PATH" \
        ALPINE_BOUNDARY_TEST_CLOSURE="$PWD/assurance/alpine-studio-dependencies.txt" \
        ALPINE_BOUNDARY_TEST_EXIT=$status ALPINE_PRODUCT_SOURCE_INPUT='' \
        ALPINE_PRODUCT_FEATURE_INPUT='' ALPINE_PRODUCT_PATH_INPUT='' \
        scripts/check-product-boundary.sh > "$fixture_dir/cargo-$status.log" 2>&1 || result=$?
    [ "$result" -eq "$status" ] || {
        cat "$fixture_dir/cargo-$status.log" >&2
        echo "product boundary lost Cargo status $status (got $result)" >&2; exit 1
    }
done

printf 'Alpine Studio product boundary tests passed\n'
