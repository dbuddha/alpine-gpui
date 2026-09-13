#!/bin/sh
set -eu
root=$(pwd)
real_cargo=$(command -v cargo)
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
mkdir -p "$temporary/repo" "$temporary/bin"
(
    cd "$temporary/repo"
    git init -q
    git config user.name 'Alpine CI fixture'
    git config user.email 'ci-fixture@example.invalid'
    printf 'initial\n' > README.md
    git add README.md && git commit -qm initial
    base=$(git rev-parse HEAD)
    printf 'policy\n' > policy.sh
    git add policy.sh && git commit -qm policy
    docs=$(git rev-parse HEAD)
    GITHUB_OUTPUT= "$root/scripts/classify-mutation-diff.sh" true "$base" "$docs" > "$temporary/empty"
    grep -qx 'required=false' "$temporary/empty"
    grep -qx 'reason=proven-empty-no-rust-paths' "$temporary/empty"
    printf 'fn value() -> bool { true }\n' > 'quoted file.rs'
    git add 'quoted file.rs' && git commit -qm rust
    rust=$(git rev-parse HEAD)
    GITHUB_OUTPUT= "$root/scripts/classify-mutation-diff.sh" true "$docs" "$rust" > "$temporary/nonempty"
    grep -qx 'required=true' "$temporary/nonempty"
    git mv 'quoted file.rs' source.txt && git commit -qm rename
    renamed=$(git rev-parse HEAD)
    GITHUB_OUTPUT= "$root/scripts/classify-mutation-diff.sh" true "$rust" "$renamed" > "$temporary/rename"
    grep -qx 'required=true' "$temporary/rename"
    GITHUB_OUTPUT= "$root/scripts/classify-mutation-diff.sh" false "$docs" "$rust" > "$temporary/unselected"
    grep -qx 'reason=not-selected' "$temporary/unselected"
    if GITHUB_OUTPUT= "$root/scripts/classify-mutation-diff.sh" true "$base" 0000000000000000000000000000000000000000 > "$temporary/invalid" 2>&1; then
        printf 'unknown source accepted as empty\n' >&2; exit 1
    fi
    if GITHUB_OUTPUT= "$root/scripts/classify-mutation-diff.sh" '' "$base" "$docs" >/dev/null 2>&1; then
        printf 'missing selection accepted\n' >&2; exit 1
    fi
)
cat > "$temporary/bin/rustc" <<'MOCK'
#!/bin/sh
printf 'rustc %s\n' "${CI_TEST_RUST_VERSION:-1.97.1}"
MOCK
cat > "$temporary/bin/cargo" <<'MOCK'
#!/bin/sh
set -eu
if [ "$1" = --version ]; then printf 'cargo 1.97.1\n'; exit 0; fi
if [ "$1" = mutants ]; then
    if [ -x "$CARGO_HOME/bin/cargo-mutants" ]; then
        exec "$CARGO_HOME/bin/cargo-mutants" "$@"
    fi
    exec cargo-mutants "$@"
fi
printf '%s\n' "$*" >> "$CI_TEST_INSTALLS"
[ "$*" != '' ]
[ "${RUSTFLAGS+x}" != x ]
[ "${CARGO_ENCODED_RUSTFLAGS+x}" != x ]
[ "${CARGO_BUILD_TARGET+x}" != x ]
[ "$1 $2 $3 $4 $5 $6" = 'install --locked cargo-mutants --version 27.1.0 --root' ]
while [ "$1" != --root ]; do shift; done
shift
mkdir -p "$1/bin"
cat > "$1/bin/cargo-mutants" <<'BINARY'
#!/bin/sh
test "$*" = 'mutants --version' || exit 96
printf 'cargo-mutants 27.1.0\n'
BINARY
chmod +x "$1/bin/cargo-mutants"
MOCK
chmod +x "$temporary/bin/cargo" "$temporary/bin/rustc"
export CI_TEST_INSTALLS="$temporary/installs" ALPINE_MUTATION_TOOL_ROOT="$temporary/tool"
export CARGO_HOME="$temporary/cargo-home"
mkdir -p "$CARGO_HOME/bin"
export ALPINE_MUTATION_CACHE_SCOPE=refs/pull/100/merge
export PATH="$temporary/bin:$PATH"
export GITHUB_PATH="$temporary/github-path"
export RUSTFLAGS=must-not-leak CARGO_ENCODED_RUSTFLAGS=must-not-leak CARGO_BUILD_TARGET=must-not-leak
: > "$CI_TEST_INSTALLS"
scripts/prepare-mutation-tool.sh > "$temporary/cold"
[ "$(wc -l < "$CI_TEST_INSTALLS" | tr -d ' ')" -eq 1 ]
grep -q 'hit=false' "$temporary/cold"
scripts/prepare-mutation-tool.sh > "$temporary/warm"
[ "$(wc -l < "$CI_TEST_INSTALLS" | tr -d ' ')" -eq 1 ]
grep -q 'hit=true' "$temporary/warm"
ALPINE_MUTATION_CACHE_SCOPE=refs/heads/main scripts/prepare-mutation-tool.sh > "$temporary/scope"
[ "$(wc -l < "$CI_TEST_INSTALLS" | tr -d ' ')" -eq 2 ]
grep -q 'hit=false' "$temporary/scope"
printf '\nprintf corrupt-executed > "%s"\n' "$temporary/unsafe-execution" >> "$ALPINE_MUTATION_TOOL_ROOT/bin/cargo-mutants"
ALPINE_MUTATION_CACHE_SCOPE=refs/heads/main scripts/prepare-mutation-tool.sh > "$temporary/corrupt"
[ ! -e "$temporary/unsafe-execution" ]
[ "$(wc -l < "$CI_TEST_INSTALLS" | tr -d ' ')" -eq 3 ]
CI_TEST_RUST_VERSION=changed ALPINE_MUTATION_CACHE_SCOPE=refs/heads/main scripts/prepare-mutation-tool.sh > "$temporary/compiler"
[ "$(wc -l < "$CI_TEST_INSTALLS" | tr -d ' ')" -eq 4 ]
printf malformed > "$ALPINE_MUTATION_TOOL_ROOT/receipt"
scripts/prepare-mutation-tool.sh > "$temporary/receipt"
[ "$(wc -l < "$CI_TEST_INSTALLS" | tr -d ' ')" -eq 5 ]
if ALPINE_MUTATION_CACHE_SCOPE= scripts/prepare-mutation-tool.sh >/dev/null 2>&1; then
    printf 'missing trust scope accepted\n' >&2; exit 1
fi
ln -s "$ALPINE_MUTATION_TOOL_ROOT" "$temporary/linked"
if ALPINE_MUTATION_TOOL_ROOT="$temporary/linked" scripts/prepare-mutation-tool.sh >/dev/null 2>&1; then
    printf 'symlink cache ownership accepted\n' >&2; exit 1
fi
grep -qx "$ALPINE_MUTATION_TOOL_ROOT/bin" "$GITHUB_PATH"
# Use the actual Cargo dispatcher, not only the stub, to establish precedence.
printf '#!/bin/sh\nprintf "foreign-tool\\n"\n' > "$CARGO_HOME/bin/cargo-mutants"
chmod +x "$CARGO_HOME/bin/cargo-mutants"
PATH="$ALPINE_MUTATION_TOOL_ROOT/bin:$PATH" "$real_cargo" mutants --version > "$temporary/real-dispatch"
grep -qx foreign-tool "$temporary/real-dispatch"
before=$(shasum -a 256 "$CARGO_HOME/bin/cargo-mutants")
if scripts/prepare-mutation-tool.sh > "$temporary/shadow-conflict" 2>&1; then
    printf 'differing Cargo-home executable was accepted\n' >&2; exit 1
fi
grep -q 'conflicting Cargo-home executable' "$temporary/shadow-conflict"
[ "$before" = "$(shasum -a 256 "$CARGO_HOME/bin/cargo-mutants")" ]
cp "$ALPINE_MUTATION_TOOL_ROOT/bin/cargo-mutants" "$CARGO_HOME/bin/cargo-mutants"
scripts/prepare-mutation-tool.sh > "$temporary/equal-dispatch"
grep -q 'hit=true' "$temporary/equal-dispatch"
if "$ALPINE_MUTATION_TOOL_ROOT/bin/cargo-mutants" --version; then
    printf 'tool fixture accepted an invalid invocation\n' >&2; exit 1
fi
printf 'CI empty-selection and mutation-tool cache controls passed\n'

# Exercise the production aggregate functions with valid and invalid domains.
awk '
    /^          require_success\(\)/ { capture = 1 }
    /^          require_success classify/ { exit }
    capture { sub(/^          /, ""); print }
' .github/workflows/ci.yml > "$temporary/aggregate.sh"
for specification in true:success:0 true:skipped:1 false:skipped:0 false:success:0 false:failure:1 :skipped:1 malformed:skipped:1; do
    required=${specification%%:*}
    remainder=${specification#*:}
    outcome=${remainder%:*}
    expected=${remainder##*:}
    actual=0
    sh -c '. "$1"; require_selected fixture "$2" "$3"' sh "$temporary/aggregate.sh" "$required" "$outcome" > "$temporary/aggregate.log" 2>&1 || actual=$?
    [ "$actual" -eq "$expected" ] || { printf 'invalid aggregate disposition: %s\n' "$specification" >&2; exit 1; }
done
printf 'CI aggregate boolean and outcome controls passed\n'

# Cargo aliases can lie about their version. Exercise real dispatch before
# requiring the preparer to reject both environment and TOML aliases.
cat > "$temporary/bin/cargo-review-other" <<'ALIAS'
#!/bin/sh
case "$*" in
    'review-other --version') printf 'cargo-mutants 27.1.0\n' ;;
    *) printf 'UNVERIFIED-ALIAS-EXECUTED\n' ;;
esac
ALIAS
chmod +x "$temporary/bin/cargo-review-other"
CARGO_ALIAS_MUTANTS=review-other "$real_cargo" mutants --version > "$temporary/alias-version"
grep -qx 'cargo-mutants 27.1.0' "$temporary/alias-version"
CARGO_ALIAS_MUTANTS=review-other "$real_cargo" mutants --list > "$temporary/alias-list"
grep -qx UNVERIFIED-ALIAS-EXECUTED "$temporary/alias-list"
if CARGO_ALIAS_MUTANTS=review-other scripts/prepare-mutation-tool.sh > "$temporary/alias-rejection" 2>&1; then
    printf 'environment alias dispatch was accepted\n' >&2; exit 1
fi
grep -q 'aliases are not admitted' "$temporary/alias-rejection"
for declaration in 'mutants = "review-other"' '"m\u0075tants" = "review-other"'; do
    printf '[alias]\n%s\n' "$declaration" > "$CARGO_HOME/config.toml"
    "$real_cargo" mutants --list > "$temporary/config-alias-list"
    grep -qx UNVERIFIED-ALIAS-EXECUTED "$temporary/config-alias-list"
    if scripts/prepare-mutation-tool.sh > "$temporary/config-alias-rejection" 2>&1; then
        printf 'TOML alias dispatch was accepted\n' >&2; exit 1
    fi
    grep -q 'aliases or unresolved includes are not admitted' "$temporary/config-alias-rejection"
done
printf '[alias]\nharmless = "check"\n' > "$CARGO_HOME/config.toml"
# A changed admitted config invalidates the prior tool identity; use an empty
# fixture Cargo home bin so foreign bytes cannot shadow the rebuilt tool.
export CARGO_HOME="$temporary/unrelated-config-home"
mkdir -p "$CARGO_HOME"
printf '[alias]\nharmless = "check"\n' > "$CARGO_HOME/config.toml"
scripts/prepare-mutation-tool.sh > "$temporary/unrelated-alias"
grep -q 'hit=false' "$temporary/unrelated-alias"
printf 'CI real Cargo alias and configuration controls passed\n'

# Cargo normalizes both unset and empty CARGO_HOME to HOME/.cargo. Keep the
# interpreter/toolchain roots stable while HOME itself is a disposable fixture.
fixture_home="$temporary/default-home"
mkdir -p "$fixture_home/.cargo"
printf '[alias]\nmutants = "review-other"\n' > "$fixture_home/.cargo/config.toml"
fixture_rustup_home=${RUSTUP_HOME:-"$HOME/.rustup"}
fixture_python_userbase=$(python3 -m site --user-base)
HOME="$fixture_home" RUSTUP_HOME="$fixture_rustup_home" CARGO_HOME= \
    "$real_cargo" mutants --list > "$temporary/empty-home-dispatch"
grep -qx UNVERIFIED-ALIAS-EXECUTED "$temporary/empty-home-dispatch"
for mode in empty unset; do
    if [ "$mode" = empty ]; then
        result=0
        HOME="$fixture_home" RUSTUP_HOME="$fixture_rustup_home" PYTHONUSERBASE="$fixture_python_userbase" CARGO_HOME= \
            scripts/prepare-mutation-tool.sh > "$temporary/default-home-rejection" 2>&1 || result=$?
    else
        result=0
        env -u CARGO_HOME HOME="$fixture_home" RUSTUP_HOME="$fixture_rustup_home" PYTHONUSERBASE="$fixture_python_userbase" \
            scripts/prepare-mutation-tool.sh > "$temporary/default-home-rejection" 2>&1 || result=$?
    fi
    [ "$result" -ne 0 ] || { printf '%s Cargo-home alias was accepted\n' "$mode" >&2; exit 1; }
    grep -q 'aliases or unresolved includes are not admitted' "$temporary/default-home-rejection"
done
printf 'CI empty and unset Cargo-home alias controls passed\n'
