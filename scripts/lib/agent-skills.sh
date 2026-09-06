#!/bin/sh
# Shared inventory reader. Validate every row before callers mutate destinations.
agent_skill_names() {
    agent_repo=$1
    agent_root=$2
    agent_manifest=$agent_root/manifest.tsv
    [ -f "$agent_manifest" ] || { printf 'agent skill manifest error: missing manifest\n' >&2; return 1; }
    agent_rows=$(awk -F '\t' '
        function fail(message) { print "agent skill manifest error: " message > "/dev/stderr"; bad=1; exit 1 }
        NR == 1 {
            if ($0 != "name\tclass\teval_suite\tcanonical_doc") fail("invalid header")
            next
        }
        {
            if (NF != 4) fail("expected four columns")
            if ($1 !~ /^[a-z0-9]+(-[a-z0-9]+)*$/ || length($1) > 63) fail("malformed name")
            if (seen[$1]++) fail("duplicate name")
            if ($2 != "github" && $2 != "engineering") fail("unknown class")
            if ($3 !~ /^assurance\/agent-skills\/v[1-9][0-9]*\/[a-z0-9-]+\.tsv$/) fail("invalid evaluation path")
            if ($4 !~ /^docs\/(operations|quality)\/[a-z0-9-]+\.md$/) fail("invalid canonical document path")
            print
            count++
        }
        END { if (!bad && count == 0) fail("empty inventory") }
    ' "$agent_manifest") || return 1
    while IFS="$(printf '\t')" read -r agent_name agent_class agent_suite agent_doc; do
        [ -d "$agent_root/$agent_name" ] && [ -f "$agent_root/$agent_name/SKILL.md" ] || {
            printf 'agent skill manifest error: missing skill folder %s\n' "$agent_name" >&2; return 1;
        }
        [ -f "$agent_repo/$agent_suite" ] || {
            printf 'agent skill manifest error: missing evaluation %s\n' "$agent_suite" >&2; return 1;
        }
        [ -f "$agent_repo/$agent_doc" ] || {
            printf 'agent skill manifest error: missing canonical document %s\n' "$agent_doc" >&2; return 1;
        }
    done <<EOF
$agent_rows
EOF
    printf '%s\n' "$agent_rows" | cut -f 1
}
