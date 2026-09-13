# Alpine engineering

Alpine is one personal Apple Silicon macOS editor: Alpine Editor on Alpine GPUI.
It aims at a smaller memory footprint than comparable editors. That aim is
unverified, so publish no comparative claim; the comparison lab is parked.
Terminal, database views, an agent dock, plugins, AI and collaboration are out
of scope.

It must behave as a real macOS application: menu bar, open and save dialogs,
multiple windows, and an installed bundle with an icon. Every surface follows
one written design spec, with Zed as the visual reference.

Daily use is the acceptance test. A defect you hit while editing is the backlog.
Judge the product by using it, not by reading its code.

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
- Report implemented, measured and daily-driver accepted separately. Keep
  outputs proportional to scope.
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
cargo run --locked -p alpine-editor
cargo test --locked -p <affected-crate>
scripts/check-native.sh physical shipping
cargo fmt --all -- --check
scripts/check.sh
```

## Standing limits

One optional skill lives in `.agents/skills`: Apple Metal for lifecycle,
presentation and residency. `docs/AGENTS.md` applies to documentation work.

This file is capped at 500 words; adding a rule requires removing one. No new
script may test another script. No workflow may file issues. Retired process is
deleted, not archived; history at tag `pre-cleanup-2026-09` restores nothing.
