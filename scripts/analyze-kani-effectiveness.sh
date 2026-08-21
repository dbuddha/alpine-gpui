#!/bin/sh
set -eu

list=${1:-target/kani/harnesses.json}
proofs=${2:-target/kani/proofs.log}
report_root=${3:-target/kani}
manifest=${KANI_CONTROL_MANIFEST:-formal/kani/effectiveness-controls.tsv}
expected_version=${KANI_EXPECTED_VERSION:-0.67.0}

fail() {
    printf 'Kani effectiveness: %s\n' "$1" >&2
    exit 1
}

test -s "$list" || fail "missing harness inventory $list"
test -s "$proofs" || fail "missing proof log $proofs"
test -s "$manifest" || fail "missing control manifest $manifest"
version=$(grep -Eo '"kani-version"[[:space:]]*:[[:space:]]*"[^"]+"' "$list" | head -n 1 | sed -E 's/.*"([^"]+)"$/\1/')
file_version=$(grep -Eo '"file-version"[[:space:]]*:[[:space:]]*"[^"]+"' "$list" | head -n 1 | sed -E 's/.*"([^"]+)"$/\1/')
test "$version" = "$expected_version" || fail "expected Kani $expected_version, found ${version:-missing}"
test "$file_version" = 0.1 || fail "expected inventory schema 0.1, found ${file_version:-missing}"

temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
if ! awk -F '\t' '
    /^[[:space:]]*#/ || /^[[:space:]]*$/ { next }
    NF != 2 || $1 == "" || $2 !~ /^[1-9][0-9]*$/ { exit 2 }
    { print $1 "\t" $2 }
' "$manifest" > "$temporary/expected.unsorted"; then
    fail "manifest rows must contain a harness and positive cover count"
fi
sort "$temporary/expected.unsorted" > "$temporary/expected"
if test "$(cut -f1 "$temporary/expected" | uniq -d | wc -l | tr -d ' ')" -ne 0; then
    fail "control manifest contains duplicate harnesses"
fi
proof_count=$(wc -l < "$temporary/expected" | tr -d ' ')
test "$proof_count" -gt 0 || fail "control manifest contains no proof harnesses"

inventory_total=$(sed -nE 's/.*"standard-harnesses":[[:space:]]*([0-9]+).*/\1/p' "$list" | tail -n 1)
contract_total=$(sed -nE 's/.*"contract-harnesses":[[:space:]]*([0-9]+).*/\1/p' "$list" | tail -n 1)
test "$inventory_total" = "$proof_count" ||
    fail "Kani inventory reports ${inventory_total:-missing} standard harnesses, manifest declares $proof_count"
test "${contract_total:-missing}" = 0 ||
    fail "contract harnesses require an explicit effectiveness policy"

grep -E '^[[:space:]]*Checking harness ' "$proofs" |
    sed -E 's/^[[:space:]]*Checking harness (.*)\.\.\.$/\1/' |
    sort -u > "$temporary/checked"
cut -f1 "$temporary/expected" > "$temporary/declared"
if ! cmp -s "$temporary/checked" "$temporary/declared"; then
    diff -u "$temporary/declared" "$temporary/checked" >&2 || true
    fail "checked harnesses do not exactly match the control manifest"
fi
checked=$(wc -l < "$temporary/checked" | tr -d ' ')

complete=$(grep -E 'Complete - [0-9]+ successfully verified harnesses, [0-9]+ failures, [0-9]+ total' "$proofs" | tail -n 1 || true)
test -n "$complete" || fail "proof log has no complete harness summary"
successful=$(printf '%s\n' "$complete" | sed -E 's/.*Complete - ([0-9]+) successfully verified harnesses, ([0-9]+) failures, ([0-9]+) total.*/\1/')
failures=$(printf '%s\n' "$complete" | sed -E 's/.*Complete - ([0-9]+) successfully verified harnesses, ([0-9]+) failures, ([0-9]+) total.*/\2/')
total=$(printf '%s\n' "$complete" | sed -E 's/.*Complete - ([0-9]+) successfully verified harnesses, ([0-9]+) failures, ([0-9]+) total.*/\3/')
test "$failures" -eq 0 || fail "$failures harnesses failed"
test "$successful" -eq "$proof_count" || fail "$successful harnesses succeeded, expected $proof_count"
test "$total" -eq "$proof_count" || fail "Kani reported $total total harnesses, expected $proof_count"
verification_successes=$(grep -Ec 'VERIFICATION:- SUCCESSFUL' "$proofs" || true)
test "$verification_successes" -eq "$proof_count" ||
    fail "$verification_successes harness result blocks succeeded, expected $proof_count"
if grep -Eq 'Status: (FAILURE|UNDETERMINED|UNKNOWN)' "$proofs"; then
    fail "proof log contains failed or undetermined properties"
fi

awk '
    /^[[:space:]]*Checking harness / {
        harness = $0
        sub(/^[[:space:]]*Checking harness /, "", harness)
        sub(/\.\.\.$/, "", harness)
        next
    }
    /^Check [0-9]+: .*\.cover\.[0-9]+/ { cover = 1; next }
    /^Check [0-9]+:/ { cover = 0; next }
    /- Status:/ && cover {
        status = $NF
        if (status != "SATISFIED" && status != "COVERED") {
            print harness "\t" status > "/dev/stderr"
            invalid = 1
        } else {
            count[harness]++
        }
        cover = 0
    }
    END {
        for (harness in count) print harness "\t" count[harness]
        if (invalid) exit 2
    }
' "$proofs" | sort > "$temporary/actual" ||
    fail "a cover obligation was unsatisfied, unreachable, or undetermined"
if ! cmp -s "$temporary/expected" "$temporary/actual"; then
    diff -u "$temporary/expected" "$temporary/actual" >&2 || true
    fail "satisfied cover counts do not exactly match the control manifest"
fi
satisfied=$(awk -F '\t' '{ total += $2 } END { print total + 0 }' "$temporary/actual")
unreachable=$(grep -Ec 'Status: UNREACHABLE' "$proofs" || true)
repository_unreachable=$(awk '
    /- Status: UNREACHABLE/ { pending = 1; next }
    pending && /- Location:/ {
        if ($0 ~ /- Location: (crates|apps|tools)\//) count++
        pending = 0
    }
    /^Check [0-9]+:/ { pending = 0 }
    END { print count + 0 }
' "$proofs")
assumptions=$(find crates apps tools -type f -name '*.rs' -exec grep -h 'kani::assume' {} + 2>/dev/null | wc -l | tr -d ' ')

mkdir -p "$report_root"
harness_rows=$report_root/effectiveness-harnesses.tsv
printf 'harness\tresult\tsatisfied_covers\n' > "$harness_rows"
awk -F '\t' '{ print $1 "\tverified\t" $2 }' "$temporary/actual" >> "$harness_rows"
revision=${REVISION:-$(git rev-parse HEAD 2>/dev/null || printf unknown)}
list_sha=$(shasum -a 256 "$list" | awk '{print $1}')
proofs_sha=$(shasum -a 256 "$proofs" | awk '{print $1}')
manifest_sha=$(shasum -a 256 "$manifest" | awk '{print $1}')
rows_sha=$(shasum -a 256 "$harness_rows" | awk '{print $1}')
cat > "$report_root/effectiveness.toml" <<EOF
schema_version = 1
revision = "$revision"
tool = "Kani"
tool_version = "$version"
inventory_schema_version = "$file_version"
proof_harnesses = $proof_count
verified_harnesses = $successful
cover_properties = $satisfied
satisfied_cover_properties = $satisfied
unreachable_checks = $unreachable
repository_unreachable_checks = $repository_unreachable
source_assumption_occurrences = $assumptions
control_manifest_sha256 = "$manifest_sha"
harness_inventory_sha256 = "$list_sha"
proof_log_sha256 = "$proofs_sha"
harness_rows_sha256 = "$rows_sha"
EOF
