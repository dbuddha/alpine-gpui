---
product: Alpine GPUI
scope: framework
execution: serial, one feature at a time, one worktree
verification: launch the installed app and look at it; reading code is not verification
parity_reference: pinned Zed v1.15.0 at alpine-zed-lab/.lab/zed
delivery: docs/delivery.md
---

# Alpine GPUI

An application framework for Apple Silicon macOS, in Rust, with a Direct Metal
backend. It exists to make one thing structurally true: an application built on
it uses far less memory and has more predictable latency than the alternatives,
because the framework refuses the designs that make those costs unbounded.

[apps/alpine-editor/AGENTS.md](apps/alpine-editor/AGENTS.md) owns the editor
built on it. This file owns the framework and the shared engineering rules.

## The contract

These are the numbers the framework exists to hit. None is currently proven.

| Property | Target |
| --- | --- |
| Matched footprint against the comparator | at least 20% lower, accepted only when the upper 95% bound on the ratio is <= 0.80 |
| CPU frame preparation | p95 <= 4 ms, p99 <= 6 ms |
| Event received through submission | p95 <= 4 ms, p99 <= 8 ms |
| Actual 120 Hz presentation | under 1% missed opportunities, where measurable |
| Idle CPU | under 1% of one core |

Honest status: the footprint advantage is **unmeasured**. An early comparison
only measured idle footprint after opening a folder, which Alpine never reads,
so it compared an idle shell against an IDE doing real work. On the one renderer
fixture with statistical treatment, pinned GPUI is about 12% faster at
`renderer-submit-readback`. On the development Mac `presentedTime` is always
zero, so presentation cannot be timed there at all (issues #511 and #622).

## Invariants that produce the contract

Violating any of these forfeits the reason the framework exists. Treat a change
that relaxes one as a product decision, not an implementation detail.

- **No reactive graph and no general async executor.** Work is demand-driven.
  Nothing recomputes because something else changed; a frame happens because an
  invalidation asked for one.
- **Everything is bounded.** Queues, caches, workers, retained bytes, in-flight
  frames. Current budgets: layout cache 32 MiB, glyph atlas 16 MiB, 3 frame
  slots, 3 overscan lines, 1 MiB per line, 64 atlas row patches before resync.
  A new allocation without a ceiling is a defect.
- **Scenes are immutable and painter order is explicit.** No implicit z-order,
  no retained mutable view tree.
- **Work is laid out for what is visible**, plus overscan. Cost must scale with
  the viewport, not the document.
- **Stale results are rejected by revision**, not by hoping they arrive in order.
- **Direct Metal in the hot path.** No generic GPU abstraction between the scene
  and the command buffer.

## Measuring anything here

- Separate the stages and never collapse them: state mutation, layout, scene
  build, adaptation, upload, encode, commit, GPU completion, presentation.
- GPU completion is not presentation. Requested bytes are not physical
  residency. Absent presentation is missing evidence, never a timestamp
  substituted from callback arrival or a target deadline.
- Memory means `phys_footprint`, summed across every process the app owns, not
  RSS. Fresh process per trial, isolated `HOME`, one launch method throughout.
- A number without a matched workload measures product scope, not efficiency.
  State what the other side was doing.
- At least ten trials, and report a confidence interval rather than a point
  estimate.

## Working rules

- Inspect branch, upstream and dirty state. Preserve unfinished work.
- Measure a differentiator before building on it. An unmeasured hypothesis
  outranks any feature.
- Read affected code and tests first. Use [docs/architecture](docs/architecture/README.md).
- Verify relevant behavior once; repeat after changes or failures. Review the
  full diff, including untracked files.
- An environmental blocker needs a re-check after a real delay before it becomes
  a blocked goal. Three reads in one minute is one observation.
- Ask about a new public contract, dependency, unsafe boundary or copied source.
- Never publish secrets, rewrite published history or bypass branch protection.

## Pitfalls with scars

- Lock and RefCell reentrancy across native callbacks, and main-thread blocking.
  Native handles, callback generations and teardown need explicit ownership.
- Unsafe code needs a local safety argument and focused tests, and lives only in
  the audited boundary files that `check-policy.sh` lists.
- Preserve blended painter order. A cross-GPU pixel hash alone proves nothing;
  native rendering needs semantic and readback checks.
- Zed application source stays in the isolated GPL lab.

## Commands

```sh
cargo run --locked -p alpine-editor
cargo test --locked -p <affected-crate>
scripts/check-native.sh physical shipping
scripts/check.sh
```

## How work is delivered

Serially. One feature at a time, one worktree, no parallel branches of work.
Phases and acceptance criteria are in [docs/delivery.md](docs/delivery.md); a
phase does not begin until the previous one passes **on main** with evidence.

Verify by using the installed application, not by reading the code that
implements it. A command reachable only from the command palette is not
shipped: that is how this project's language-server work sat unusable for
months while looking complete in source.

Each AGENTS.md is capped at 900 words; adding a rule requires removing one. No
new script may test another script. No workflow may file issues. Retired process
is deleted, not archived; history at tag `pre-cleanup-2026-09` restores nothing.
