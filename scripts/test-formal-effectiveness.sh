#!/bin/sh
set -eu
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT HUP INT TERM
mkdir -p "$fixture/kani"

assert_fails() {
    description=$1
    shift
    if "$@" > "$fixture/failure.log" 2>&1; then
        printf 'invalid formal-effectiveness fixture passed: %s\n' "$description" >&2
        exit 1
    fi
}

cat > "$fixture/kani/harnesses.json" <<'EOF'
{"kani-version":"0.67.0","file-version":"0.1","standard-harnesses":{"fixture.rs":["example::bounded_property"]},"contract-harnesses":{},"contracts":[],"totals":{"standard-harnesses":1,"contract-harnesses":0,"functions-under-contract":0}}
EOF
printf 'example::bounded_property\t2\n' > "$fixture/kani/effectiveness-controls.tsv"
cat > "$fixture/kani/proofs.log" <<'EOF'
Checking harness example::bounded_property...
RESULTS:
Check 1: bounded_property.cover.1
         - Status: SATISFIED
Check 2: bounded_property.cover.2
         - Status: SATISFIED
SUMMARY:
 ** 2 of 2 cover properties satisfied
VERIFICATION:- SUCCESSFUL
Complete - 1 successfully verified harnesses, 0 failures, 1 total.
EOF
KANI_CONTROL_MANIFEST="$fixture/kani/effectiveness-controls.tsv" REVISION=fixture \
    scripts/analyze-kani-effectiveness.sh "$fixture/kani/harnesses.json" "$fixture/kani/proofs.log" "$fixture/kani/report"
grep -Fq 'proof_harnesses = 1' "$fixture/kani/report/effectiveness.toml"
grep -Fq 'satisfied_cover_properties = 2' "$fixture/kani/report/effectiveness.toml"

awk '!changed && /SATISFIED/ { sub(/SATISFIED/, "UNSATISFIABLE"); changed = 1 } { print }' \
    "$fixture/kani/proofs.log" > "$fixture/kani/proofs-bad.log"
assert_fails 'unsatisfied Kani cover' env KANI_CONTROL_MANIFEST="$fixture/kani/effectiveness-controls.tsv" scripts/analyze-kani-effectiveness.sh "$fixture/kani/harnesses.json" "$fixture/kani/proofs-bad.log" "$fixture/kani/report"

printf 'example::bounded_property\t1\n' > "$fixture/kani/effectiveness-controls-wrong.tsv"
assert_fails 'wrong Kani cover count' env KANI_CONTROL_MANIFEST="$fixture/kani/effectiveness-controls-wrong.tsv" scripts/analyze-kani-effectiveness.sh "$fixture/kani/harnesses.json" "$fixture/kani/proofs.log" "$fixture/kani/report"

printf '%s\n' 'example::bounded_property\t2' > "$fixture/kani/effectiveness-controls-literal-escape.tsv"
assert_fails 'literal escape instead of Kani manifest tab' env KANI_CONTROL_MANIFEST="$fixture/kani/effectiveness-controls-literal-escape.tsv" scripts/analyze-kani-effectiveness.sh "$fixture/kani/harnesses.json" "$fixture/kani/proofs.log" "$fixture/kani/report"
