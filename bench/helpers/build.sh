#!/bin/sh
set -eu

# Builds the Swift helpers and the AppKit reference editor with Command Line
# Tools (xcrun swiftc), runs their GUI-free self-tests and writes a stamp.

if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
    printf 'bench helper build error: Apple Silicon macOS is required\n' >&2
    exit 1
fi

bench_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
helpers="$bench_root/helpers"
output="$bench_root/target/helpers"
mkdir -p "$output"
# Keep Spotlight and the Open dialog away from build products.
: > "$bench_root/target/.metadata_never_index"

compile() {
    name=$1
    shift
    xcrun swiftc -O -parse-as-library -swift-version 5 -warnings-as-errors \
        "$@" -o "$output/$name"
    printf 'built %s\n' "$output/$name"
}

compile bench-window "$helpers/Common.swift" "$helpers/Window.swift"
compile bench-sample "$helpers/Common.swift" "$helpers/Sample.swift"
compile bench-input "$helpers/Common.swift" "$helpers/Input.swift"
compile bench-capture "$helpers/Common.swift" "$helpers/Capture.swift"
compile bench-reference-appkit "$bench_root/reference-appkit/ReferenceEditor.swift"

for helper in bench-window bench-sample bench-input bench-capture; do
    "$output/$helper" self-test >/dev/null
done
"$output/bench-reference-appkit" --self-test >/dev/null
printf 'helper self-tests passed\n'

stamp="$output/stamp.tsv"
{
    printf 'swiftc\t%s\n' "$(xcrun swiftc --version 2>&1 | head -n 1)"
    printf 'sdk\t%s\n' "$(xcrun --show-sdk-version)"
    for source in "$helpers"/*.swift "$bench_root"/reference-appkit/*.swift; do
        printf 'source\t%s\t%s\n' "${source#"$bench_root"/}" \
            "$(shasum -a 256 "$source" | awk '{print $1}')"
    done
} > "$stamp.tmp"
mv "$stamp.tmp" "$stamp"
printf 'wrote %s\n' "$stamp"
