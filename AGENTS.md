# Alpine engineering

Two products live here, both for Apple Silicon macOS only.

**Alpine GPUI** (`crates/`) is the framework: safe Rust contracts, an immutable
scene protocol, a demand-driven bounded runtime with no reactive graph and no
general async executor, hard-budgeted caches, explicit resource accounting, and
a Direct Metal backend. Its objective is predictable latency and bounded memory
under explicit ownership, which is what makes a low footprint structural rather
than incidental. The programming model is conceptually adapted from Zed GPUI and
independently written; no upstream source is copied, vendored or linked. Out of
scope: Intel, Linux, Windows, web, mobile, GPUI source compatibility, and a
generic GPU abstraction in the Metal hot path.

**Alpine Editor** (`apps/alpine-editor`) is the application. Its goals, product
bar and design rules are in [apps/alpine-editor/AGENTS.md](apps/alpine-editor/AGENTS.md).

## Working rules

- Inspect branch, upstream and dirty state. Preserve unfinished work. Fetch
  before comparing remote branches.
- Measure a differentiator before building on it. An unmeasured hypothesis
  outranks any feature.
- The user request or an existing issue supplies scope. State the observable
  outcome, the failure it fixes and its regression check. Honor authorization
  already given.
- Read affected code and tests first. Use [docs/architecture](docs/architecture/README.md),
  then only the relevant topic. Do not bulk-read docs.
- Verify relevant behavior once; repeat after changes or failures. Review the
  full diff, including untracked files.
- Report implemented, measured and accepted separately. Keep outputs
  proportional to scope.
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

One optional skill is in `.agents/skills`: Apple Metal for lifecycle,
presentation and residency. `docs/AGENTS.md` applies to documentation work.

Each AGENTS.md is capped at 500 words; adding a rule requires removing one. No
new script may test another script. No workflow may file issues. Retired process
is deleted, not archived; history at tag `pre-cleanup-2026-09` restores nothing.
