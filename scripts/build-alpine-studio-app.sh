#!/bin/sh
set -eu

usage() {
    cat <<'EOF'
usage: scripts/build-alpine-studio-app.sh [--executable PATH] [--output PATH]

Build and assemble the local release Alpine Studio.app. Supplying an executable
skips Cargo and is intended for structural validation or an already-built
release binary. The output basename must remain "Alpine Studio.app".
EOF
}

repository_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
output="$repository_root/target/release/Alpine Studio.app"
executable=

while [ "$#" -gt 0 ]; do
    case "$1" in
        --executable)
            [ "$#" -ge 2 ] || {
                printf 'app bundle error: --executable requires a path\n' >&2
                exit 2
            }
            executable=$2
            shift 2
            ;;
        --output)
            [ "$#" -ge 2 ] || {
                printf 'app bundle error: --output requires a path\n' >&2
                exit 2
            }
            output=$2
            shift 2
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            printf 'app bundle error: unsupported argument %s\n' "$1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

case "$output" in
    */'Alpine Studio.app'|'Alpine Studio.app') ;;
    *)
        printf 'app bundle error: output basename must be Alpine Studio.app\n' >&2
        exit 2
        ;;
esac

if [ -z "$executable" ]; then
    if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
        printf 'app bundle error: release builds require Apple Silicon macOS\n' >&2
        exit 1
    fi
    (cd "$repository_root" && cargo build --release --locked -p alpine-studio)
    executable="$repository_root/target/release/alpine-studio"
fi

if [ ! -f "$executable" ] || [ ! -x "$executable" ]; then
    printf 'app bundle error: executable is missing or not executable: %s\n' \
        "$executable" >&2
    exit 1
fi

workspace_version=$(sed -nE \
    's/^version = "([0-9]+\.[0-9]+\.[0-9]+)"$/\1/p' \
    "$repository_root/Cargo.toml" | head -n 1)
if [ -z "$workspace_version" ]; then
    printf 'app bundle error: workspace version is not plain SemVer\n' >&2
    exit 1
fi

if [ -n "${ALPINE_BUNDLE_FIXTURE_REVISION-}" ]; then
    revision=$ALPINE_BUNDLE_FIXTURE_REVISION
else
    if [ -n "$(git -C "$repository_root" status --porcelain --untracked-files=normal)" ]; then
        printf 'app bundle error: revision-pinned bundle requires a clean worktree\n' >&2
        exit 1
    fi
    revision=$(git -C "$repository_root" rev-parse HEAD)
fi
case "$revision" in
    *[!0-9a-f]*|'')
        printf 'app bundle error: revision must be a lowercase hexadecimal commit\n' >&2
        exit 1
        ;;
esac
if [ "${#revision}" -ne 40 ]; then
    printf 'app bundle error: revision must contain exactly 40 hexadecimal digits\n' >&2
    exit 1
fi

output_parent=$(dirname "$output")
mkdir -p "$output_parent"
staging_dir=$(mktemp -d "$output_parent/.alpine-studio-app.XXXXXX")
cleanup() {
    if [ -n "${staging_dir-}" ] && [ -d "$staging_dir" ]; then
        rm -rf "$staging_dir"
    fi
}
trap cleanup EXIT HUP INT TERM

bundle="$staging_dir/Alpine Studio.app"
contents="$bundle/Contents"
macos="$contents/MacOS"
resources="$contents/Resources"
mkdir -p "$macos" "$resources"
cp "$executable" "$macos/alpine-studio"
chmod 0755 "$macos/alpine-studio"

cat > "$contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleDisplayName</key>
    <string>Alpine Studio</string>
    <key>CFBundleExecutable</key>
    <string>alpine-studio</string>
    <key>CFBundleIdentifier</key>
    <string>com.dbuddha.alpine-studio</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>Alpine Studio</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$workspace_version</string>
    <key>CFBundleVersion</key>
    <string>$workspace_version</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.developer-tools</string>
    <key>LSMinimumSystemVersion</key>
    <string>15.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF

binary_sha256=$(shasum -a 256 "$macos/alpine-studio" | awk '{print $1}')
binary_bytes=$(wc -c < "$macos/alpine-studio" | tr -d ' ')
plist_sha256=$(shasum -a 256 "$contents/Info.plist" | awk '{print $1}')
cat > "$resources/alpine-build-identity.toml" <<EOF
schema = "alpine-studio-dogfood-bundle/v1"
revision = "$revision"
workspace_version = "$workspace_version"
build_profile = "release"
target = "aarch64-apple-darwin"
bundle_identifier = "com.dbuddha.alpine-studio"
executable_sha256 = "$binary_sha256"
executable_bytes = $binary_bytes
info_plist_sha256 = "$plist_sha256"
signed = false
EOF

if command -v plutil >/dev/null 2>&1; then
    plutil -lint "$contents/Info.plist" >/dev/null
fi

backup="$output_parent/.Alpine Studio.app.previous.$$"
if [ -e "$backup" ]; then
    printf 'app bundle error: replacement backup already exists: %s\n' "$backup" >&2
    exit 1
fi
if [ -e "$output" ]; then
    mv "$output" "$backup"
fi
if mv "$bundle" "$output"; then
    staging_dir=
    if [ -e "$backup" ]; then
        rm -rf "$backup"
    fi
else
    if [ -e "$backup" ]; then
        mv "$backup" "$output"
    fi
    printf 'app bundle error: failed to publish assembled bundle\n' >&2
    exit 1
fi

printf '%s\n' "$output"
