#!/bin/sh
set -eu

repo_root=${1:-$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)}
catalog=$repo_root/docs/research/index.md
review=$repo_root/docs/research/alpine-studio-adversarial-review.md
comparator=$repo_root/docs/quality/comparator-protocol.md
studio_path=$repo_root/docs/use-cases/alpine-studio-highfidelity.md
zed_editor=$repo_root/docs/case-studies/zed-editor.md
zed_gpui=$repo_root/docs/case-studies/zed-gpui.md
sublime=$repo_root/docs/case-studies/sublime-editor.md
failures=0

fail() {
    printf 'research retention error: %s\n' "$1" >&2
    failures=$((failures + 1))
}

for required in \
    "$catalog" \
    "$review" \
    "$comparator" \
    "$studio_path" \
    "$zed_editor" \
    "$zed_gpui" \
    "$sublime"
do
    if [ ! -f "$required" ]; then
        fail "required research artifact is missing: ${required#"$repo_root"/}"
    fi
done

if [ "$failures" -ne 0 ]; then
    exit 1
fi

for link in \
    '(alpine-studio-adversarial-review.md)' \
    '(../case-studies/zed-editor.md)' \
    '(../case-studies/zed-gpui.md)' \
    '(../case-studies/sublime-editor.md)' \
    '(../quality/comparator-protocol.md)' \
    '(../use-cases/alpine-studio-highfidelity.md)'
do
    if ! grep -Fq "$link" "$catalog"; then
        fail "research catalog is missing canonical link $link"
    fi
done

link_errors=$(mktemp)
trap 'rm -f "$link_errors"' EXIT HUP INT TERM
for source in \
    "$catalog" \
    "$review" \
    "$comparator" \
    "$studio_path" \
    "$zed_editor" \
    "$zed_gpui" \
    "$sublime"
do
    grep -Eo '\]\([^)]+\)' "$source" 2>/dev/null \
        | sed 's/^](//; s/)$//' \
        | while IFS= read -r link; do
            case "$link" in
                http://*|https://*|mailto:*|'#'*) continue ;;
            esac
            target=${link%%#*}
            target=${target%%\?*}
            if [ -n "$target" ] && [ ! -e "$(dirname "$source")/$target" ]; then
                printf '%s -> %s\n' "${source#"$repo_root"/}" "$link" >> "$link_errors"
            fi
        done
done
if [ -s "$link_errors" ]; then
    fail 'repository-relative research links do not resolve'
    cat "$link_errors" >&2
fi

for requirement in 32 33 34 35 36 37; do
    if ! grep -Fq "https://github.com/dbuddha/alpine-gpui/issues/$requirement" "$review"; then
        fail "adversarial review is missing research anchor for Requirement #$requirement"
    fi
done

for issue in 113 114 115 116 118 132; do
    if ! grep -Fq "https://github.com/dbuddha/alpine-gpui/issues/$issue" "$catalog"; then
        fail "research catalog is missing issue anchor #$issue"
    fi
done

for field in workload_identity_hash environment_hash exclusion_manifest_hash; do
    if ! grep -Fq "$field" "$comparator"; then
        fail "comparator protocol is missing mandatory field $field"
    fi
done

for heading in \
    '## Adaptation separation' \
    '## Explicit exclusion manifest' \
    '## Correctness admission' \
    '## Invalid runs' \
    '## Claim grammar'
do
    if ! grep -Fqx "$heading" "$comparator"; then
        fail "comparator protocol is missing required section: $heading"
    fi
done

if [ "$failures" -ne 0 ]; then
    exit 1
fi

printf 'research retention catalog and evidence chain are valid\n'
