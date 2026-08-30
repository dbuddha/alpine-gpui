#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
wiki=$repo_root/scripts/wiki.sh
revision=0123456789abcdef0123456789abcdef01234567
temporary=$(mktemp -d "${TMPDIR:-/tmp}/alpine-wiki-test.XXXXXX")
trap 'rm -rf "$temporary"' EXIT HUP INT TERM

expect_failure() {
    name=$1
    shift
    if "$@" >"$temporary/$name.out" 2>&1; then
        printf 'expected failure: %s\n' "$name" >&2
        exit 1
    fi
}

"$wiki" validate "$repo_root"
"$wiki" render "$revision" "$temporary/render-a" "$repo_root"
"$wiki" render "$revision" "$temporary/render-b" "$repo_root"
[ "$(find "$temporary/render-a" -type f -name '*.md' | wc -l | tr -d ' ')" = 12 ]
diff -ru "$temporary/render-a" "$temporary/render-b"
grep -F "$revision" "$temporary/render-a/Home.md" >/dev/null
expect_failure invalid-revision "$wiki" render short "$temporary/invalid" "$repo_root"

cp -R "$repo_root/docs" "$temporary/missing-source-docs"
rm "$temporary/missing-source-docs/case-studies/wgpu.md"
mkdir "$temporary/missing-source-root"
mv "$temporary/missing-source-docs" "$temporary/missing-source-root/docs"
expect_failure missing-source "$wiki" validate "$temporary/missing-source-root"

cp -R "$repo_root/docs" "$temporary/broken-link-docs"
mkdir "$temporary/broken-link-root"
mv "$temporary/broken-link-docs" "$temporary/broken-link-root/docs"
printf '\n[Unknown](Unknown-Page)\n' >> "$temporary/broken-link-root/docs/wiki/pages/Home.md"
expect_failure unknown-link "$wiki" validate "$temporary/broken-link-root"

source_root=$temporary/source
mkdir "$source_root"
cp -R "$repo_root/docs" "$source_root/docs"
git -C "$source_root" init -q -b main
git -C "$source_root" config user.name 'Alpine Wiki Test'
git -C "$source_root" config user.email 'wiki-test@example.invalid'
git -C "$source_root" add docs
git -C "$source_root" commit -qm 'test source'
git -C "$source_root" update-ref refs/remotes/origin/main HEAD

wiki_root=$temporary/wiki
mkdir "$wiki_root"
git -C "$wiki_root" init -q
git -C "$wiki_root" remote add origin https://github.com/dbuddha/alpine-gpui.wiki.git
"$wiki" publish "$wiki_root" "$source_root" >/dev/null
[ "$(find "$wiki_root" -type f -name '*.md' | wc -l | tr -d ' ')" = 12 ]

wiki_remote=$temporary/wiki-remote.git
git init -q --bare --initial-branch=master "$wiki_remote"
git -C "$wiki_root" config user.name 'Alpine Wiki Test'
git -C "$wiki_root" config user.email 'wiki-test@example.invalid'
git -C "$wiki_root" add .
git -C "$wiki_root" commit -qm 'test wiki'
git -C "$wiki_root" push -q "$wiki_remote" HEAD:master
wiki_audit=$temporary/wiki-audit
GIT_ALLOW_PROTOCOL=file git clone -q "$wiki_remote" "$wiki_audit"
git -C "$wiki_audit" remote set-url origin https://github.com/dbuddha/alpine-gpui.wiki.git
git -C "$wiki_audit" config url."file://$wiki_remote".insteadOf https://github.com/dbuddha/alpine-gpui.wiki.git
GIT_ALLOW_PROTOCOL=file "$wiki" audit-remote "$wiki_audit" "$source_root" >/dev/null

printf '\n' >> "$source_root/docs/wiki/README.md"
git -C "$source_root" add docs/wiki/README.md
git -C "$source_root" commit -qm 'advance source revision'
git -C "$source_root" update-ref refs/remotes/origin/main HEAD
expect_failure stale-live-wiki env GIT_ALLOW_PROTOCOL=file "$wiki" audit-remote "$wiki_audit" "$source_root"

printf '# unknown\n' > "$wiki_root/Unknown.md"
expect_failure unknown-destination "$wiki" publish "$wiki_root" "$source_root"
rm "$wiki_root/Unknown.md"

git -C "$source_root" switch -qc topic
expect_failure topic-branch "$wiki" publish "$wiki_root" "$source_root"
git -C "$source_root" switch -q main
printf '\n' >> "$source_root/docs/wiki/README.md"
expect_failure dirty-source "$wiki" publish "$wiki_root" "$source_root"

printf 'Wiki mirror tests passed.\n'
