#!/bin/sh
set -eu

list=${1:-target/kani/harnesses.json}
proofs=${2:-target/kani/proofs.log}
report_root=${3:-target/kani}
source_root=${KANI_SOURCE_ROOT:-.}
expected_version=${KANI_EXPECTED_VERSION:-0.67.0}

fail() {
    printf 'Kani effectiveness: %s\n' "$1" >&2
    exit 1
}

test -s "$list" || fail "missing harness inventory $list"
test -s "$proofs" || fail "missing proof log $proofs"
version=$(grep -Eo '"kani-version"[[:space:]]*:[[:space:]]*"[^"]+"' "$list" | head -n 1 | sed -E 's/.*"([^"]+)"$/\1/')
test -n "$version" || fail "harness inventory has no kani-version"
test "$version" = "$expected_version" || fail "expected Kani $expected_version, found $version"

proof_count=$(rg -o '#\[kani::proof\]' "$source_root/crates" "$source_root/apps" "$source_root/tools" --glob '*.rs' 2>/dev/null | wc -l | tr -d ' ')
cover_count=$(rg -o 'kani::cover!' "$source_root/crates" "$source_root/apps" "$source_root/tools" --glob '*.rs' 2>/dev/null | wc -l | tr -d ' ')
assumption_count=$(rg -o 'kani::assume' "$source_root/crates" "$source_root/apps" "$source_root/tools" --glob '*.rs' 2>/dev/null | wc -l | tr -d ' ')
test "$proof_count" -gt 0 || fail "source contains no proof harnesses"
checked=$(grep -E '^[[:space:]]*Checking harness ' "$proofs" | sed -E 's/^[[:space:]]*Checking harness ([^.]|\.[^.])*\.\.\.$/\1/' | sort -u | wc -l | tr -d ' ')
test "$checked" -eq "$proof_count" || fail "checked $checked harnesses, source declares $proof_count"

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
if grep -Eq 'Status: (FAILURE|UNREACHABLE|UNDETERMINED|UNKNOWN|UNSATISFIABLE|UNCOVERED)' "$proofs"; then
    fail "proof log contains failed, unreachable, or undetermined properties"
fi
satisfied=$(grep -Ec 'Status: (SATISFIED|COVERED)' "$proofs" || true)
test "$satisfied" -eq "$cover_count" || fail "$satisfied cover properties satisfied, source declares $cover_count"

mkdir -p "$report_root"
harness_rows=$report_root/effectiveness-harnesses.tsv
printf 'harness\tresult\n' > "$harness_rows"
grep -E '^[[:space:]]*Checking harness ' "$proofs" | sed -E 's/^[[:space:]]*Checking harness (.*)\.\.\.$/\1\tverified/' | sort -u >> "$harness_rows"
revision=${REVISION:-$(git rev-parse HEAD 2>/dev/null || printf unknown)}
list_sha=$(shasum -a 256 "$list" | awk '{print $1}')
proofs_sha=$(shasum -a 256 "$proofs" | awk '{print $1}')
rows_sha=$(shasum -a 256 "$harness_rows" | awk '{print $1}')
cat > "$report_root/effectiveness.toml" <<EOF
schema_version = 1
revision = "$revision"
tool = "Kani"
tool_version = "$version"
proof_harnesses = $proof_count
verified_harnesses = $successful
cover_properties = $cover_count
satisfied_cover_properties = $satisfied
assumptions = $assumption_count
harness_inventory_sha256 = "$list_sha"
proof_log_sha256 = "$proofs_sha"
harness_rows_sha256 = "$rows_sha"
EOF
