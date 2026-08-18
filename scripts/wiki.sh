#!/bin/sh
set -eu

fail() {
    printf 'wiki: %s\n' "$*" >&2
    exit 1
}

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
default_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
expected_pages='Home
Execution-Map
Research-Index
Zed
Sublime-Text
WGPU
Comparator-Qualification
Agent-Operations
Documentation-Policy
_Sidebar'

root_for() {
    if [ "$#" -eq 0 ]; then
        printf '%s\n' "$default_root"
    else
        CDPATH= cd -- "$1" && pwd
    fi
}

manifest_pages() {
    cut -f1 "$1/docs/wiki/manifest.tsv"
}

is_expected_page() {
    page=$1
    printf '%s\n' "$expected_pages" | grep -Fx -- "$page" >/dev/null 2>&1
}

validate_source() {
    root=$1
    manifest=$root/docs/wiki/manifest.tsv
    pages=$root/docs/wiki/pages
    [ -f "$root/docs/wiki/README.md" ] || fail 'missing docs/wiki/README.md'
    [ -f "$manifest" ] || fail 'missing docs/wiki/manifest.tsv'
    [ -d "$pages" ] || fail 'missing docs/wiki/pages'

    actual=$(manifest_pages "$root")
    [ "$actual" = "$expected_pages" ] || fail 'manifest page inventory is not the approved bounded set'
    [ "$(printf '%s\n' "$actual" | sort -u | wc -l | tr -d ' ')" = 10 ] || fail 'manifest contains duplicate pages'

    while IFS="$(printf '\t')" read -r page title source tracking extra; do
        [ -n "$page" ] || fail 'manifest contains an empty page name'
        [ -n "$title" ] || fail "manifest title is empty for $page"
        [ -z "${extra:-}" ] || fail "manifest has extra fields for $page"
        is_expected_page "$page" || fail "unapproved page in manifest: $page"
        [ -f "$pages/$page.md" ] || fail "missing page template: $page.md"
        [ -f "$root/$source" ] || fail "missing canonical source for $page: $source"
        grep -F '{{ALPINE_MAIN_REVISION}}' "$pages/$page.md" >/dev/null || fail "missing revision marker in $page.md"
        old_ifs=$IFS
        IFS=,
        for issue in $tracking; do
            case "$issue" in
                https://github.com/dbuddha/alpine-gpui/issues/[0-9]*) ;;
                *) fail "invalid tracking issue for $page: $issue" ;;
            esac
        done
        IFS=$old_ifs
    done < "$manifest"

    for template in "$pages"/*.md; do
        page=$(basename "$template" .md)
        is_expected_page "$page" || fail "unapproved page template: $page.md"
        links=$(grep -Eo '\]\([A-Za-z0-9_-]+\)' "$template" 2>/dev/null || true)
        if [ -n "$links" ]; then
            printf '%s\n' "$links" | sed -e 's/^](//' -e 's/)$//' | while IFS= read -r target; do
                is_expected_page "$target" || fail "unknown Wiki link in $page.md: $target"
            done
        fi
    done
}

validate_revision() {
    revision=$1
    [ "${#revision}" -eq 40 ] || fail 'revision must be a full 40-character hash'
    case "$revision" in
        *[!0-9a-f]*) fail 'revision must be a lowercase hexadecimal hash' ;;
    esac
}

render() {
    revision=$1
    output=$2
    root=$3
    validate_revision "$revision"
    validate_source "$root"
    if [ -e "$output" ]; then
        [ -d "$output" ] || fail 'render output exists and is not a directory'
        [ -z "$(find "$output" -type f -print -quit)" ] || fail 'render output must be empty'
    else
        mkdir -p "$output"
    fi

    while IFS="$(printf '\t')" read -r page title source tracking extra; do
        {
            printf '<!-- Generated from %s at %s. Do not edit the Wiki directly. -->\n\n' "$source" "$revision"
            sed "s/{{ALPINE_MAIN_REVISION}}/$revision/g" "$root/docs/wiki/pages/$page.md"
            printf '\n---\nCanonical path: `%s`  \nTracking: ' "$source"
            old_ifs=$IFS
            IFS=,
            first=1
            for issue in $tracking; do
                if [ "$first" -eq 0 ]; then printf ', '; fi
                printf '[%s](%s)' "$(basename "$issue")" "$issue"
                first=0
            done
            IFS=$old_ifs
            printf '\n'
        } > "$output/$page.md"
    done < "$root/docs/wiki/manifest.tsv"
}

publish() {
    wiki_root=$(CDPATH= cd -- "$1" && pwd)
    root=$2
    validate_source "$root"
    command -v git >/dev/null 2>&1 || fail 'git is required for publication'
    [ -z "$(git -C "$root" status --porcelain)" ] || fail 'source checkout must be clean'
    [ "$(git -C "$root" symbolic-ref --short HEAD 2>/dev/null || true)" = main ] || fail 'source checkout must be on main'
    head=$(git -C "$root" rev-parse HEAD)
    upstream=$(git -C "$root" rev-parse origin/main 2>/dev/null || true)
    [ -n "$upstream" ] && [ "$head" = "$upstream" ] || fail 'source HEAD must equal origin/main'
    remote=$(git -C "$wiki_root" remote get-url origin 2>/dev/null || true)
    [ "$remote" = 'https://github.com/dbuddha/alpine-gpui.wiki.git' ] || fail 'Wiki checkout has an unexpected origin URL'

    for existing in "$wiki_root"/*.md; do
        [ -e "$existing" ] || continue
        page=$(basename "$existing" .md)
        is_expected_page "$page" || fail "refusing unknown destination page: $page.md"
    done

    temporary=$(mktemp -d "${TMPDIR:-/tmp}/alpine-wiki.XXXXXX")
    trap 'rm -rf "$temporary"' EXIT HUP INT TERM
    render "$head" "$temporary" "$root"
    for generated in "$temporary"/*.md; do
        cp "$generated" "$wiki_root/$(basename "$generated")"
    done
    printf 'Wiki pages rendered from %s. Review, commit, and push the Wiki checkout separately.\n' "$head"
}

usage() {
    cat >&2 <<'EOF'
usage:
  scripts/wiki.sh validate [REPO_ROOT]
  scripts/wiki.sh render REVISION OUTPUT [REPO_ROOT]
  scripts/wiki.sh publish WIKI_ROOT [REPO_ROOT]
EOF
    exit 2
}

[ "$#" -ge 1 ] || usage
command=$1
shift
case "$command" in
    validate)
        [ "$#" -le 1 ] || usage
        root=$(root_for "$@")
        validate_source "$root"
        ;;
    render)
        [ "$#" -ge 2 ] && [ "$#" -le 3 ] || usage
        revision=$1
        output=$2
        shift 2
        root=$(root_for "$@")
        render "$revision" "$output" "$root"
        ;;
    publish)
        [ "$#" -ge 1 ] && [ "$#" -le 2 ] || usage
        wiki_root=$1
        shift
        root=$(root_for "$@")
        publish "$wiki_root" "$root"
        ;;
    *) usage ;;
esac
