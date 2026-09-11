#!/bin/sh
set -eu

root=$(pwd)
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
mkdir "$temporary/bin"
cat > "$temporary/bin/cargo" <<'EOF'
#!/bin/sh
set -eu
printf 'cargo %s\n' "$*" >> "$ADMISSION_CALLS"
if [ "$1" = test ] && [ "${ALPINE_RUST_ANALYZER+x}" = x ]; then
    printf 'fixture: analyzer must be absent from native admission\n' >&2
    exit 91
fi
if [ "$1" = test ]; then
    test "${RUSTFLAGS:-}" = '--cfg alpine_native_validation' || exit 92
    test "${ALPINE_PRESENTATION_EVIDENCE_MODE:-}" = hosted-direct || exit 93
    test "${MACOSX_DEPLOYMENT_TARGET:-}" = 15.0 || exit 94
    test "${ALPINE_VALIDATION_DEPLOYMENT_TARGET:-}" = 26.0 || exit 95
fi
case "${ADMISSION_FAULT:-}:$*" in
    format:fmt*) exit 31 ;;
    clippy:clippy*) exit 32 ;;
    studio:'test --locked --package=alpine-studio') exit 33 ;;
    platform:'test --locked --package=alpine-platform-macos --package=alpine-studio') exit 34 ;;
esac
EOF
cat > "$temporary/bin/xcrun" <<'EOF'
#!/bin/sh
set -eu
printf 'xcrun %s\n' "$*" >> "$ADMISSION_CALLS"
case "${ADMISSION_FAULT:-}:$*" in
    toolchain:'--sdk macosx --find metal') exit 1 ;;
    metallib:'--sdk macosx --find metallib') exit 35 ;;
esac
EOF
cat > "$temporary/bin/xcodebuild" <<'EOF'
#!/bin/sh
set -eu
printf 'xcodebuild %s\n' "$*" >> "$ADMISSION_CALLS"
exit 36
EOF
chmod +x "$temporary/bin/cargo" "$temporary/bin/xcrun" "$temporary/bin/xcodebuild"

run_case() {
    name=$1
    script=$2
    fault=$3
    expected=$4
    : > "$temporary/$name.calls"
    actual=0
    PATH="$temporary/bin:$PATH" ADMISSION_CALLS="$temporary/$name.calls" \
        ADMISSION_FAULT="$fault" ALPINE_RUST_ANALYZER=must-be-removed \
        RUSTFLAGS="${ADMISSION_TEST_RUSTFLAGS:---cfg alpine_native_validation}" \
        ALPINE_PRESENTATION_EVIDENCE_MODE=hosted-direct \
        MACOSX_DEPLOYMENT_TARGET=15.0 ALPINE_VALIDATION_DEPLOYMENT_TARGET=26.0 \
        "$root/scripts/$script" > "$temporary/$name.log" 2>&1 || actual=$?
    if [ "$actual" -ne "$expected" ]; then
        printf 'CI admission test error: %s expected exit %s, got %s\n' \
            "$name" "$expected" "$actual" >&2
        cat "$temporary/$name.log" >&2
        exit 1
    fi
}

run_case fast-pass check-ci-fast-feedback.sh '' 0
printf '%s\n' 'cargo fmt --all -- --check' \
    'cargo clippy --workspace --all-targets --all-features --locked -- -D warnings' \
    > "$temporary/fast.expected"
diff -u "$temporary/fast.expected" "$temporary/fast-pass.calls"
run_case format-fail check-ci-fast-feedback.sh format 31
printf '%s\n' 'cargo fmt --all -- --check' > "$temporary/format.expected"
diff -u "$temporary/format.expected" "$temporary/format-fail.calls"
run_case clippy-fail check-ci-fast-feedback.sh clippy 32
diff -u "$temporary/fast.expected" "$temporary/clippy-fail.calls"
if grep -q 'CI fast feedback passed' "$temporary/format-fail.log" "$temporary/clippy-fail.log"; then
    printf 'CI admission test error: failed fast feedback published success\n' >&2
    exit 1
fi

run_case native-pass check-ci-native-admission.sh '' 0
printf '%s\n' 'xcrun --sdk macosx --find metal' \
    'xcrun --sdk macosx --find metal' 'xcrun --sdk macosx --find metallib' \
    'cargo test --locked --package=alpine-studio' \
    'cargo test --locked --package=alpine-platform-macos --package=alpine-studio' \
    > "$temporary/native.expected"
diff -u "$temporary/native.expected" "$temporary/native-pass.calls"
run_case studio-fail check-ci-native-admission.sh studio 33
sed '$d' "$temporary/native.expected" > "$temporary/studio.expected"
diff -u "$temporary/studio.expected" "$temporary/studio-fail.calls"
run_case platform-fail check-ci-native-admission.sh platform 34
diff -u "$temporary/native.expected" "$temporary/platform-fail.calls"
run_case toolchain-fail check-ci-native-admission.sh toolchain 36
printf '%s\n' 'xcrun --sdk macosx --find metal' \
    'xcodebuild -downloadComponent MetalToolchain' > "$temporary/toolchain.expected"
diff -u "$temporary/toolchain.expected" "$temporary/toolchain-fail.calls"
run_case metallib-fail check-ci-native-admission.sh metallib 35
sed '/^cargo /d' "$temporary/native.expected" > "$temporary/metallib.expected"
diff -u "$temporary/metallib.expected" "$temporary/metallib-fail.calls"
( ADMISSION_TEST_RUSTFLAGS=ordinary-build run_case missing-validation check-ci-native-admission.sh '' 2 )
test ! -s "$temporary/missing-validation.calls"
grep -q 'explicit hosted validation environment required' "$temporary/missing-validation.log"
for name in studio-fail platform-fail toolchain-fail metallib-fail missing-validation; do
    if grep -q 'CI native admission passed' "$temporary/$name.log"; then
        printf 'CI admission test error: %s published native success\n' "$name" >&2
        exit 1
    fi
done
# Remove cfg only after the production entry guard. The actual command boundary
# must discriminate this fault, rather than merely checking the launch fixture.
sed '/^unset ALPINE_RUST_ANALYZER/a\
unset RUSTFLAGS
' "$root/scripts/check-ci-native-admission.sh" > "$temporary/cfg-loss.sh"
chmod +x "$temporary/cfg-loss.sh"
original_root=$root
mkdir "$temporary/scripts"
cp "$temporary/cfg-loss.sh" "$temporary/scripts/cfg-loss.sh"
root=$temporary
run_case cfg-loss cfg-loss.sh '' 92
root=$original_root
if grep -q 'CI native admission passed' "$temporary/cfg-loss.log"; then
    printf 'CI admission test error: lost command cfg published success\n' >&2
    exit 1
fi
printf 'CI admission ordering and failure-propagation controls passed\n'
