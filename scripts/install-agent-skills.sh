#!/bin/sh
set -eu
fail() { printf 'agent skill install error: %s\n' "$1" >&2; exit 1; }
usage() { printf 'usage: scripts/install-agent-skills.sh --check|--install|--remove-links\n' >&2; exit 2; }
[ "$#" -eq 1 ] || usage
action=$1
case "$action" in --check|--install|--remove-links) ;; *) usage ;; esac
repo_root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
source_root=$repo_root/skills
destination_root=${CODEX_HOME:-"$HOME/.codex"}/skills
skills='github-project-operator github-documentation-architect github-deep-researcher'
for skill in $skills; do [ -f "$source_root/$skill/SKILL.md" ] || fail "missing source skill $skill"; done
if [ "$action" = '--check' ]; then
    for skill in $skills; do
        destination=$destination_root/$skill
        [ -L "$destination" ] || fail "$destination is not an installed repository link"
        [ "$(readlink "$destination")" = "$source_root/$skill" ] || fail "$destination does not point to this repository"
    done
    printf 'repository agent skill links are installed\n'; exit 0
fi
if [ "$action" = '--install' ]; then
    for skill in $skills; do
        destination=$destination_root/$skill
        if [ -e "$destination" ] || [ -L "$destination" ]; then
            [ -L "$destination" ] || fail "refusing to replace $destination"
            [ "$(readlink "$destination")" = "$source_root/$skill" ] || fail "refusing to replace unrelated link $destination"
        fi
    done
    mkdir -p "$destination_root"
    for skill in $skills; do destination=$destination_root/$skill; [ -L "$destination" ] || ln -s "$source_root/$skill" "$destination"; done
    printf 'repository agent skill links installed in %s\n' "$destination_root"; exit 0
fi
for skill in $skills; do
    destination=$destination_root/$skill
    if [ -e "$destination" ] || [ -L "$destination" ]; then
        [ -L "$destination" ] || fail "refusing to remove $destination"
        [ "$(readlink "$destination")" = "$source_root/$skill" ] || fail "refusing to remove unrelated link $destination"
    fi
done
for skill in $skills; do destination=$destination_root/$skill; [ ! -L "$destination" ] || unlink "$destination"; done
printf 'repository agent skill links removed from %s\n' "$destination_root"
