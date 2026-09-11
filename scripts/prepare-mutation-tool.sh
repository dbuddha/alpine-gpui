#!/bin/sh
set -eu

# Cache only tool inputs. No test outcome, inventory or acceptance is reused.
version=27.1.0
root=${ALPINE_MUTATION_TOOL_ROOT:-"$HOME/.cache/alpine-ci/cargo-mutants"}
scope=${ALPINE_MUTATION_CACHE_SCOPE:?explicit cache trust scope required}
case "$scope" in *'
'*) printf 'invalid cache scope\n' >&2; exit 2 ;; esac
for path in "$root" "$root/bin" "$root/bin/cargo-mutants" "$root/receipt"; do
    [ ! -L "$path" ] || { printf 'refusing symlink cache entry: %s\n' "$path" >&2; exit 2; }
done
temporary=$(mktemp -d "${TMPDIR:-/tmp}/alpine-mutants.XXXXXX")
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
# Stable Cargo has no resolved-config query. Parse its ancestor/home config
# inputs conservatively instead; aliases may otherwise redirect a version probe
# and every later `cargo mutants` call to different bytes.
python3 - <<'PY' > "$temporary/config-identity"
import hashlib
import os
from pathlib import Path
try:
    import tomllib
except ImportError:
    import tomli as tomllib

if "CARGO_ALIAS_MUTANTS" in os.environ:
    raise SystemExit("mutation tool error: Cargo mutants aliases are not admitted")
cwd = Path.cwd()
directories = [path / ".cargo" for path in (cwd, *cwd.parents)]
directories.append(Path(os.environ.get("CARGO_HOME") or str(Path.home() / ".cargo")))
seen = set()
for directory in directories:
    legacy = directory / "config"
    path = legacy if legacy.exists() else directory / "config.toml"
    if not path.exists() or path.resolve() in seen:
        continue
    seen.add(path.resolve())
    content = path.read_bytes()
    config = tomllib.loads(content.decode("utf-8"))
    aliases = config.get("alias", {})
    environment = config.get("env", {})
    if not isinstance(aliases, dict) or not isinstance(environment, dict):
        raise SystemExit("mutation tool error: invalid Cargo dispatch configuration")
    if "mutants" in aliases or "CARGO_ALIAS_MUTANTS" in environment or "include" in config:
        raise SystemExit("mutation tool error: Cargo mutants aliases or unresolved includes are not admitted")
    print(hashlib.sha256(content).hexdigest(), path.resolve())
PY
{
    printf 'version=%s\nscope=%s\n' "$version" "$scope"
    rustc -vV
    cargo --version
    uname -smr
    cat "$temporary/config-identity"
    shasum -a 256 "$0" | awk '{print $1}'
} > "$temporary/identity"
identity=$(shasum -a 256 "$temporary/identity" | awk '{print $1}')
write_receipt() {
    printf 'schema=1\nscope=%s\nidentity=%s\nbinary=%s\nversion=%s\n' \
        "$scope" "$identity" "$1" "$version"
}
hit=false
if [ -x "$root/bin/cargo-mutants" ] && [ -f "$root/receipt" ]; then
    digest=$(shasum -a 256 "$root/bin/cargo-mutants" | awk '{print $1}')
    write_receipt "$digest" > "$temporary/expected"
    if cmp -s "$temporary/expected" "$root/receipt"; then
        # Check digest and identity before executing restored bytes. This is
        # corruption detection within GitHub's scoped cache trust boundary,
        # not authentication against a writer who controls both files.
        if [ "$("$root/bin/cargo-mutants" mutants --version)" = "cargo-mutants $version" ]; then
            hit=true
        fi
    fi
fi
if [ "$hit" = false ]; then
    printf 'mutation tool: cold install or rejected cache\n'
    env -u RUSTFLAGS -u CARGO_ENCODED_RUSTFLAGS -u CARGO_BUILD_TARGET \
        cargo install --locked cargo-mutants --version "$version" --root "$temporary/install"
    [ "$("$temporary/install/bin/cargo-mutants" mutants --version)" = "cargo-mutants $version" ]
    mkdir -p "$root/bin"
    cp "$temporary/install/bin/cargo-mutants" "$root/bin/cargo-mutants.pending"
    mv "$root/bin/cargo-mutants.pending" "$root/bin/cargo-mutants"
    digest=$(shasum -a 256 "$root/bin/cargo-mutants" | awk '{print $1}')
    write_receipt "$digest" > "$root/receipt.pending"
    mv "$root/receipt.pending" "$root/receipt"
else
    printf 'mutation tool: compatible verified cache hit\n'
fi
# Cargo checks CARGO_HOME/bin before PATH for external subcommands. A verified
# PATH entry alone therefore does not bind the bytes used by `cargo mutants`.
# Preserve a foreign installation, but reject differing bytes before execution.
cargo_home_binary=${CARGO_HOME:-"$HOME/.cargo"}/bin/cargo-mutants
if [ -e "$cargo_home_binary" ] || [ -L "$cargo_home_binary" ]; then
    if [ ! -x "$cargo_home_binary" ] || \
        [ "$(shasum -a 256 "$cargo_home_binary" | awk '{print $1}')" != "$digest" ]; then
        printf 'mutation tool error: conflicting Cargo-home executable; foreign path preserved\n' >&2
        exit 1
    fi
fi
resolved=$(PATH="$root/bin:$PATH" command -v cargo-mutants)
[ "$(shasum -a 256 "$resolved" | awk '{print $1}')" = "$digest" ]
[ "$(PATH="$root/bin:$PATH" cargo mutants --version)" = "cargo-mutants $version" ]
if [ -n "${GITHUB_PATH:-}" ]; then
    printf '%s\n' "$root/bin" >> "$GITHUB_PATH"
fi
printf 'mutation tool identity=%s digest=%s hit=%s\n' "$identity" "$digest" "$hit"
