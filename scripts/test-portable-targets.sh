#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
checker="$repo_root/scripts/check-portable-targets.sh"
targets='x86_64-unknown-linux-gnu x86_64-pc-windows-msvc'
installed=$(rustup target list --installed)
temporary=$(mktemp -d "${TMPDIR:-/tmp}/alpine-portable-targets.XXXXXX")
trap 'rm -rf "$temporary"' EXIT HUP INT TERM

for target in $targets; do
    if ! printf '%s\n' "$installed" | rg -q "^${target}$"; then
        printf 'portable-target test requires: rustup target add %s\n' "$target" >&2
        exit 1
    fi
    rg -q --fixed-strings "$target" "$checker" || {
        printf 'portable checker omitted target %s\n' "$target" >&2
        exit 1
    }
done
rg -q --fixed-strings 'rustup target add %s' "$checker" || {
    printf 'portable checker lacks the actionable missing-target instruction\n' >&2
    exit 1
}

workflow="$repo_root/.github/workflows/ci.yml"
rg -q --fixed-strings 'portable: ${{ steps.classify.outputs.portable }}' "$workflow" || {
    printf 'CI workflow does not publish the portable classifier output\n' >&2
    exit 1
}
rg -q --fixed-strings "if: matrix.name == 'macos-arm64' || needs.classify.outputs.portable == 'true'" "$workflow" || {
    printf 'CI workflow does not guard the portable native test step\n' >&2
    exit 1
}

mkdir "$temporary/fake-bin"
cat > "$temporary/fake-bin/rustup" <<'EOF'
#!/bin/sh
printf 'x86_64-unknown-linux-gnu\n'
EOF
chmod +x "$temporary/fake-bin/rustup"
if PATH="$temporary/fake-bin:$PATH" "$checker" \
    > "$temporary/missing-target.log" 2>&1; then
    printf 'portable checker accepted a missing Windows target\n' >&2
    exit 1
fi
rg -q --fixed-strings \
    'install it with the official command: rustup target add x86_64-pc-windows-msvc' \
    "$temporary/missing-target.log" || {
    printf 'portable checker missing-target diagnostic drifted\n' >&2
    exit 1
}

cat > "$temporary/invalid.rs" <<'EOF'
#[cfg(not(target_os = "macos"))]
fn helper() {}

#[cfg(not(target_os = "macos"))]
fn main() -> Result<(), ()> {
    helper()?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn main() {}
EOF

cat > "$temporary/corrected.rs" <<'EOF'
#[cfg(not(target_os = "macos"))]
fn helper() {}

#[cfg(not(target_os = "macos"))]
fn main() {
    helper();
}

#[cfg(target_os = "macos")]
fn main() {}
EOF

for target in $targets; do
    mkdir "$temporary/invalid-$target" "$temporary/corrected-$target"
    if rustc --edition=2024 --target "$target" "$temporary/invalid.rs" \
        --emit=metadata --out-dir "$temporary/invalid-$target" >/dev/null 2>&1; then
        printf 'invalid non-macOS fixture compiled for %s\n' "$target" >&2
        exit 1
    fi
    rustc --edition=2024 --target "$target" "$temporary/corrected.rs" \
        --emit=metadata --out-dir "$temporary/corrected-$target"
done

printf 'portable target policy regression tests passed\n'
