# Alpine engineering

Alpine is one personal Apple Silicon macOS editor: Alpine Editor on Alpine GPUI,
with a memory footprint Zed cannot match. Ownership and comprehension are the
product. No comparative performance claim is pursued and the comparison lab is
parked. The executable is still `alpine-studio`. Terminal, database views, an
agent dock, multi-window, plugins, AI and collaboration are out of scope.

Daily use is the acceptance test. A defect you hit while editing is the backlog.

## Working rules

- Inspect branch, upstream and dirty state. Preserve unfinished work. Fetch
  before comparing remote branches.
- Measure the differentiator before building on it. An unmeasured hypothesis
  outranks any feature.
- The user request or an existing issue supplies scope. State the observable
  outcome, the failure it fixes and its regression check. Honor authorization
  already given.
- Read affected code and tests first. Use `ARCHITECTURE.md`, then only the
  relevant topic. Do not bulk-read docs.
- Verify relevant behavior once; repeat after changes or failures. Review the
  full diff, including untracked files.
- Report implemented, reproduced, measured and daily-driver accepted separately.
  Keep outputs proportional to scope.
- An environmental blocker needs a re-check after a real delay before it becomes
  a blocked goal. Three reads in one minute is one observation.
- Ask about a new public contract, dependency, unsafe boundary, license or
  copied source when not already authorized.
- Never publish secrets, overwrite unrelated work, rewrite published history or
  bypass branch protection.

## Concrete pitfalls

- Reject stale worker results with document and workspace revisions. Preserve
  Unicode, IME, unsaved documents and save durability across lifecycle events.
- Avoid lock and RefCell reentrancy across native callbacks, and main-thread
  blocking. Native handles, callback generations, in-flight resources and
  teardown need explicit ownership. Unsafe code needs a local safety argument
  and focused tests.
- Preserve blended painter order. Native rendering needs semantic and readback
  checks; a cross-GPU pixel hash alone is insufficient.
- GPU completion is not presentation. Requested bytes are not physical
  residency. Absent presentation is missing evidence, never a timestamp to
  substitute from callback arrival or a target deadline.
- A sub-millisecond mutation stage with tens of milliseconds to presentation
  does not justify rewriting the rope, renderer or runtime.
- Zed application source stays in the isolated GPL lab.

## Commands

```sh
cargo run --locked -p alpine-studio
cargo test --locked -p <affected-crate>
scripts/check-native.sh physical shipping
cargo fmt --all -- --check
scripts/check.sh
```

## Standing limits

One optional skill lives in `.agents/skills`: Apple Metal for lifecycle,
presentation and residency. `docs/AGENTS.md` applies to documentation work.

This file is capped at 500 words; adding a rule requires removing one. No new
script may test another script; `test-policy.sh` and `test-classifier.sh` are the
last two and retire with `check-policy.sh`. No workflow may file issues. Retired
process is deleted rather than archived: history at tag `pre-cleanup-2026-09` is
the only record, and it restores nothing.
