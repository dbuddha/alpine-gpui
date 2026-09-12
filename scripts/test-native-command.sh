#!/bin/sh
set -eu
repo_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT HUP INT TERM
mkdir -p "$fixture/repo/scripts" "$fixture/bin"
cp "$repo_root/scripts/check-native.sh" "$fixture/repo/scripts/"
cat > "$fixture/bin/uname" <<'EOF'
#!/bin/sh
case "$1" in -s) echo "${FIXTURE_OS:-Darwin}" ;; -m) echo arm64 ;; esac
EOF
cat > "$fixture/bin/cargo" <<'EOF'
#!/bin/sh
set -eu
test "$1" = test
test "$2" = --locked
test "$ALPINE_REQUIRE_NATIVE_VALIDATION" = 1
case "$RUSTFLAGS" in *'--cfg alpine_native_validation'*) ;; *) exit 98 ;; esac
test -z "${ALPINE_STUDIO_NATIVE_ACCESSIBILITY_CHILD:-}"
test -z "${ALPINE_STUDIO_NATIVE_LSP_SERVER:-}"
test "${ALPINE_PRESENTATION_EVIDENCE_MODE:-physical}" = "$EXPECTED_MODE"
test "${ALPINE_STUDIO_NATIVE_PROCESS_SCOPE:-all}" = "$EXPECTED_SCOPE"
case "$EXPECTED_SCOPE" in
 shipping) test "$3" = --package=alpine-studio; test "$4" = --test=native_process ;;
 all) test "$#" = 7; test "$7" = --all-targets ;;
esac
if [ "${FIXTURE_RESULT:-0}" != 0 ]; then echo 'retained-native-fixture-failure'; exit "$FIXTURE_RESULT"; fi
if [ "${FIXTURE_RECEIPT:-yes}" = yes ]; then echo "alpine-native-process-complete scope=$EXPECTED_SCOPE"; fi
EOF
chmod +x "$fixture/bin/"* "$fixture/repo/scripts/check-native.sh"
export PATH="$fixture/bin:$PATH"
unset CARGO_ENCODED_RUSTFLAGS CARGO_BUILD_TARGET
run() { "$fixture/repo/scripts/check-native.sh" "$@" >"$fixture/result" 2>&1; }
export EXPECTED_MODE=physical EXPECTED_SCOPE=shipping
# An inherited hosted/child environment must not alter a requested physical run.
ALPINE_PRESENTATION_EVIDENCE_MODE=hosted-direct ALPINE_STUDIO_NATIVE_ACCESSIBILITY_CHILD=1 run physical shipping
export EXPECTED_MODE=hosted-direct EXPECTED_SCOPE=all
run hosted all
if FIXTURE_RECEIPT=no run hosted all; then echo 'missing native receipt accepted' >&2; exit 1; fi
grep -q 'completion receipt missing' "$fixture/result"
if FIXTURE_RESULT=42 run hosted all; then exit 1; else result=$?; fi
test "$result" = 42
grep -q 'retained-native-fixture-failure' "$fixture/result"
grep -l 'retained-native-fixture-failure' "$fixture/repo/target/native-acceptance/"* >/dev/null
if FIXTURE_OS=Linux run physical; then echo 'unsupported platform accepted' >&2; exit 1; fi
if CARGO_ENCODED_RUSTFLAGS=override run physical; then echo 'encoded flags accepted' >&2; exit 1; fi
if CARGO_BUILD_TARGET=x86_64-apple-darwin run physical; then echo 'cross target accepted' >&2; exit 1; fi
if run invalid; then echo 'invalid evidence mode accepted' >&2; exit 1; fi
if run physical invalid; then echo 'invalid scope accepted' >&2; exit 1; fi
echo 'native command execution, omission and failure controls passed'
