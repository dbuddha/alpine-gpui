#!/bin/sh
set -eu

fixture=target/formal-effectiveness-fixture
rm -rf "$fixture"
mkdir -p "$fixture/formal/tla/aep-test" "$fixture/logs/aep-test" "$fixture/kani"
trap 'rm -rf "$fixture"' EXIT HUP INT TERM

cat > "$fixture/formal/tla/aep-test/Example.tla" <<'EOF'
---- MODULE Example ----
====
EOF
printf 'INVARIANT ExpectedInvariant\n' > "$fixture/formal/tla/aep-test/PullRequest.cfg"
printf 'INVARIANT ExpectedInvariant\n' > "$fixture/formal/tla/aep-test/Faulty.cfg"
printf 'aep-test\tFaulty.cfg\tExpectedInvariant\n' > "$fixture/formal/tla/effectiveness-controls.tsv"
cat > "$fixture/logs/aep-test/PullRequest.cfg.log" <<'EOF'
Model checking completed. No error has been found.
1,234 states generated, 456 distinct states found, 0 states left on queue.
The depth of the complete state graph search is 9.
EOF
cat > "$fixture/logs/aep-test/Faulty.cfg.log" <<'EOF'
Error: Invariant ExpectedInvariant is violated.
12 states generated, 8 distinct states found, 3 states left on queue.
EOF

TLA_MODEL_ROOT="$fixture/formal/tla" \
TLA_CONTROL_MANIFEST="$fixture/formal/tla/effectiveness-controls.tsv" \
REVISION=fixture scripts/analyze-tla-effectiveness.sh pull-request "$fixture/logs" "$fixture/tla-report"
grep -Fq 'positive_models = 1' "$fixture/tla-report/effectiveness.toml"
grep -Fq 'negative_controls = 1' "$fixture/tla-report/effectiveness.toml"
grep -Fq '1234' "$fixture/tla-report/effectiveness.tsv"

assert_fails() {
    description=$1
    shift
    if "$@" > "$fixture/failure.log" 2>&1; then
        printf 'invalid formal-effectiveness fixture passed: %s\n' "$description" >&2
        exit 1
    fi
}

sed 's/ExpectedInvariant/WrongInvariant/' "$fixture/logs/aep-test/Faulty.cfg.log" > "$fixture/logs/aep-test/Faulty.cfg.log.tmp"
mv "$fixture/logs/aep-test/Faulty.cfg.log.tmp" "$fixture/logs/aep-test/Faulty.cfg.log"
assert_fails 'wrong TLA+ invariant' env TLA_MODEL_ROOT="$fixture/formal/tla" TLA_CONTROL_MANIFEST="$fixture/formal/tla/effectiveness-controls.tsv" scripts/analyze-tla-effectiveness.sh pull-request "$fixture/logs" "$fixture/tla-report"
sed 's/WrongInvariant/ExpectedInvariant/' "$fixture/logs/aep-test/Faulty.cfg.log" > "$fixture/logs/aep-test/Faulty.cfg.log.tmp"
mv "$fixture/logs/aep-test/Faulty.cfg.log.tmp" "$fixture/logs/aep-test/Faulty.cfg.log"

sed 's/0 states left/1 states left/' "$fixture/logs/aep-test/PullRequest.cfg.log" > "$fixture/logs/aep-test/PullRequest.cfg.log.tmp"
mv "$fixture/logs/aep-test/PullRequest.cfg.log.tmp" "$fixture/logs/aep-test/PullRequest.cfg.log"
assert_fails 'unfinished TLA+ queue' env TLA_MODEL_ROOT="$fixture/formal/tla" TLA_CONTROL_MANIFEST="$fixture/formal/tla/effectiveness-controls.tsv" scripts/analyze-tla-effectiveness.sh pull-request "$fixture/logs" "$fixture/tla-report"
sed 's/1 states left/0 states left/' "$fixture/logs/aep-test/PullRequest.cfg.log" > "$fixture/logs/aep-test/PullRequest.cfg.log.tmp"
mv "$fixture/logs/aep-test/PullRequest.cfg.log.tmp" "$fixture/logs/aep-test/PullRequest.cfg.log"

printf 'aep-test\tMissing.cfg\tExpectedInvariant\n' >> "$fixture/formal/tla/effectiveness-controls.tsv"
assert_fails 'incomplete TLA+ control inventory' env TLA_MODEL_ROOT="$fixture/formal/tla" TLA_CONTROL_MANIFEST="$fixture/formal/tla/effectiveness-controls.tsv" scripts/analyze-tla-effectiveness.sh --check-controls
scripts/analyze-tla-effectiveness.sh --check-controls formal/tla formal/tla/effectiveness-controls.tsv

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
