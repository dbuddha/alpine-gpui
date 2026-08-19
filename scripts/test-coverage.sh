#!/bin/sh
set -eu

repository_root=$(pwd)
checker=$repository_root/scripts/check-coverage.sh
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' 0 HUP INT TERM

fixture_repository=$fixture/repository
git init -q "$fixture_repository"
cd "$fixture_repository"
git config user.name 'Alpine Coverage Fixture'
git config user.email 'coverage-fixture@localhost'

mkdir -p crates/demo/src apps/demo/src
printf 'pub fn existing_crate() {}\n' > crates/demo/src/lib.rs
printf 'pub fn existing_app() {}\n' > apps/demo/src/lib.rs
git add crates/demo/src/lib.rs apps/demo/src/lib.rs
git commit -q -m base
base_sha=$(git rev-parse HEAD)

cat >> crates/demo/src/lib.rs <<'EOF'
pub fn changed_crate() {}
// Changed non-executable crate line.
EOF
cat >> apps/demo/src/lib.rs <<'EOF'
pub fn changed_app() {}
// Changed non-executable app line.
EOF
git add crates/demo/src/lib.rs apps/demo/src/lib.rs
git commit -q -m head

base_summary=$fixture/base-summary.json
head_summary=$fixture/head-summary.json
base_lcov=$fixture/base.info
head_lcov=$fixture/head.info
failure_output=$fixture/failure.out
success_output=$fixture/success.out
: > "$base_summary"
: > "$base_lcov"

cat > "$head_summary" <<'EOF'
{
  "data": [{
    "totals": {
      "lines": {"percent": 100},
      "functions": {"percent": 100}
    },
    "files": [
      {"filename": "/fixture/crates/alpine-core/src/lib.rs", "summary": {"lines": {"percent": 100}, "functions": {"percent": 100}}},
      {"filename": "/fixture/crates/alpine-core/src/geometry.rs", "summary": {"lines": {"percent": 100}, "functions": {"percent": 100}}},
      {"filename": "/fixture/crates/alpine-scene/src/lib.rs", "summary": {"lines": {"percent": 100}, "functions": {"percent": 100}}},
      {"filename": "/fixture/crates/alpine-metal/src/lib.rs", "summary": {"lines": {"percent": 100}, "functions": {"percent": 100}}},
      {"filename": "/fixture/crates/alpine-platform/src/lib.rs", "summary": {"lines": {"percent": 100}, "functions": {"percent": 100}}},
      {"filename": "/fixture/crates/alpine-platform/src/event.rs", "summary": {"lines": {"percent": 100}, "functions": {"percent": 100}}}
    ]
  }]
}
EOF

cat > "$head_lcov" <<EOF
SF:$fixture_repository/crates/demo/src/lib.rs
DA:1,1
DA:2,1
end_of_record
SF:$fixture_repository/apps/demo/src/lib.rs
DA:1,1
DA:2,0
end_of_record
EOF

if ALPINE_HEAD_SHA=HEAD "$checker" \
    "$base_summary" "$head_summary" "$base_lcov" "$head_lcov" "$base_sha" \
    > "$failure_output" 2>&1; then
    printf 'coverage test error: uncovered application line was accepted\n' >&2
    exit 1
fi

expected_line='coverage error: uncovered changed executable Rust line: apps/demo/src/lib.rs:2'
if ! grep -Fxq "$expected_line" "$failure_output"; then
    printf 'coverage test error: missing exact application diagnostic\n' >&2
    cat "$failure_output" >&2
    exit 1
fi
if ! grep -Fxq 'coverage error: changed executable Rust lines covered 1/2, requires 90%' "$failure_output"; then
    printf 'coverage test error: missing exact changed-line summary\n' >&2
    cat "$failure_output" >&2
    exit 1
fi
if grep -Fq 'crates/demo/src/lib.rs:2' "$failure_output"; then
    printf 'coverage test error: covered crate line was reported as uncovered\n' >&2
    cat "$failure_output" >&2
    exit 1
fi

sed 's/DA:2,0/DA:2,1/' "$head_lcov" > "$fixture/covered.info"
head_lcov=$fixture/covered.info
ALPINE_HEAD_SHA=HEAD "$checker" \
    "$base_summary" "$head_summary" "$base_lcov" "$head_lcov" "$base_sha" \
    > "$success_output" 2>&1

if ! grep -Fxq 'coverage gates passed: lines 100.00%, functions 100.00%' "$success_output"; then
    printf 'coverage test error: covered application and crate fixture did not pass\n' >&2
    cat "$success_output" >&2
    exit 1
fi

printf 'coverage verifier tests passed\n'
