#!/bin/sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
fixture_dir=$(mktemp -d)
trap 'rm -rf "$fixture_dir"' EXIT HUP INT TERM

fake_executable="$fixture_dir/alpine-studio"
cat > "$fake_executable" <<'EOF'
#!/bin/sh
exit 0
EOF
chmod 0755 "$fake_executable"

revision=0123456789abcdef0123456789abcdef01234567
first_output="$fixture_dir/first/Alpine Studio.app"
second_output="$fixture_dir/second/Alpine Studio.app"

ALPINE_BUNDLE_FIXTURE_REVISION=$revision \
    "$repository_root/scripts/build-alpine-studio-app.sh" \
    --executable "$fake_executable" --output "$first_output" >/dev/null
ALPINE_BUNDLE_FIXTURE_REVISION=$revision \
    "$repository_root/scripts/build-alpine-studio-app.sh" \
    --executable "$fake_executable" --output "$second_output" >/dev/null

first_contents="$first_output/Contents"
first_binary="$first_contents/MacOS/alpine-studio"
first_plist="$first_contents/Info.plist"
first_identity="$first_contents/Resources/alpine-build-identity.toml"

[ -x "$first_binary" ]
cmp -s "$fake_executable" "$first_binary"
diff -r "$first_output" "$second_output" >/dev/null
grep -Fq '<string>com.dbuddha.alpine-studio</string>' "$first_plist"
grep -Fq '<string>alpine-studio</string>' "$first_plist"
grep -Fq '<string>15.0</string>' "$first_plist"
grep -Fq "revision = \"$revision\"" "$first_identity"
grep -Fq 'build_profile = "release"' "$first_identity"
grep -Fq 'target = "aarch64-apple-darwin"' "$first_identity"
grep -Fq 'signed = false' "$first_identity"

expected_sha=$(shasum -a 256 "$fake_executable" | awk '{print $1}')
expected_bytes=$(wc -c < "$fake_executable" | tr -d ' ')
grep -Fq "executable_sha256 = \"$expected_sha\"" "$first_identity"
grep -Fq "executable_bytes = $expected_bytes" "$first_identity"

if command -v plutil >/dev/null 2>&1; then
    plutil -lint "$first_plist" >/dev/null
fi

cat > "$fake_executable" <<'EOF'
#!/bin/sh
printf 'replacement\n'
EOF
chmod 0755 "$fake_executable"
ALPINE_BUNDLE_FIXTURE_REVISION=$revision \
    "$repository_root/scripts/build-alpine-studio-app.sh" \
    --executable "$fake_executable" --output "$first_output" >/dev/null
cmp -s "$fake_executable" "$first_binary"
if cmp -s "$first_binary" "$second_output/Contents/MacOS/alpine-studio"; then
    printf 'app bundle test error: replacement retained the old executable\n' >&2
    exit 1
fi

if ALPINE_BUNDLE_FIXTURE_REVISION=invalid \
    "$repository_root/scripts/build-alpine-studio-app.sh" \
    --executable "$fake_executable" --output "$first_output" \
    > "$fixture_dir/revision.log" 2>&1; then
    printf 'app bundle test error: invalid revision unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'revision must be a lowercase hexadecimal commit' \
    "$fixture_dir/revision.log"

if ALPINE_BUNDLE_FIXTURE_REVISION=$revision \
    "$repository_root/scripts/build-alpine-studio-app.sh" \
    --executable "$fake_executable" --output "$fixture_dir/not-an-app" \
    > "$fixture_dir/output.log" 2>&1; then
    printf 'app bundle test error: unsafe output basename unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'output basename must be Alpine Studio.app' "$fixture_dir/output.log"

fixture_repository="$fixture_dir/repository"
fixture_bin="$fixture_dir/bin"
mkdir -p "$fixture_repository/scripts" "$fixture_bin"
cp "$repository_root/scripts/build-alpine-studio-app.sh" \
    "$fixture_repository/scripts/build-alpine-studio-app.sh"
cat > "$fixture_repository/Cargo.toml" <<'EOF'
[workspace]

[workspace.package]
version = "0.0.0"
EOF
cat > "$fixture_bin/uname" <<'EOF'
#!/bin/sh
case "$1" in
    -s) printf 'Darwin\n' ;;
    -m) printf 'arm64\n' ;;
    *) exit 2 ;;
esac
EOF
cat > "$fixture_bin/cargo" <<'EOF'
#!/bin/sh
set -eu
[ "$*" = 'build --release --locked -p alpine-studio' ] || exit 2
case "${CARGO_TARGET_DIR-}" in
    /*) target_directory=$CARGO_TARGET_DIR ;;
    '') exit 2 ;;
    *) target_directory="$PWD/$CARGO_TARGET_DIR" ;;
esac
mkdir -p "$target_directory/release"
cat > "$target_directory/release/alpine-studio" <<'INNER'
#!/bin/sh
printf 'isolated build\n'
INNER
chmod 0755 "$target_directory/release/alpine-studio"
EOF
chmod 0755 "$fixture_bin/uname" "$fixture_bin/cargo"

default_executable="$fixture_repository/target/release/alpine-studio"
default_bundle_executable="$fixture_repository/target/release/Alpine Studio.app/Contents/MacOS/alpine-studio"
mkdir -p "$(dirname "$default_executable")" "$(dirname "$default_bundle_executable")"
printf 'stale default executable\n' > "$default_executable"
printf 'stale default bundle\n' > "$default_bundle_executable"
default_executable_sha=$(shasum -a 256 "$default_executable" | awk '{print $1}')
default_bundle_sha=$(shasum -a 256 "$default_bundle_executable" | awk '{print $1}')

absolute_target="$fixture_dir/absolute-target"
PATH="$fixture_bin:$PATH" \
    CARGO_TARGET_DIR="$absolute_target" \
    ALPINE_BUNDLE_FIXTURE_REVISION=$revision \
    "$fixture_repository/scripts/build-alpine-studio-app.sh" \
    > "$fixture_dir/absolute-target.log"
absolute_target=$(CDPATH= cd -- "$absolute_target" && pwd -P)
absolute_bundle="$absolute_target/release/Alpine Studio.app"
[ "$(cat "$fixture_dir/absolute-target.log")" = "$absolute_bundle" ]
cmp -s "$absolute_target/release/alpine-studio" \
    "$absolute_bundle/Contents/MacOS/alpine-studio"

relative_target=isolated-relative-target
PATH="$fixture_bin:$PATH" \
    CARGO_TARGET_DIR=$relative_target \
    ALPINE_BUNDLE_FIXTURE_REVISION=$revision \
    "$fixture_repository/scripts/build-alpine-studio-app.sh" \
    > "$fixture_dir/relative-target.log"
relative_target_path=$(CDPATH= cd -- "$fixture_repository/$relative_target" && pwd -P)
relative_bundle="$relative_target_path/release/Alpine Studio.app"
[ "$(cat "$fixture_dir/relative-target.log")" = "$relative_bundle" ]
cmp -s "$relative_target_path/release/alpine-studio" \
    "$relative_bundle/Contents/MacOS/alpine-studio"

[ "$(shasum -a 256 "$default_executable" | awk '{print $1}')" = \
    "$default_executable_sha" ]
[ "$(shasum -a 256 "$default_bundle_executable" | awk '{print $1}')" = \
    "$default_bundle_sha" ]

"$repository_root/scripts/build-alpine-studio-app.sh" --help >/dev/null
"$repository_root/scripts/launch-alpine-studio-app.sh" --help >/dev/null
if "$repository_root/scripts/launch-alpine-studio-app.sh" first second \
    > "$fixture_dir/launch.log" 2>&1; then
    printf 'app bundle test error: multiple launch paths unexpectedly passed\n' >&2
    exit 1
fi
grep -Fq 'expected at most one file or folder' "$fixture_dir/launch.log"

printf 'Alpine Studio app bundle contract checks passed\n'
