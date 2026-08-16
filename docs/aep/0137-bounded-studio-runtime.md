# AEP 0137: Bounded single-window Studio runtime

- Status: accepted 2026-08-15
- Capability: [#28](https://github.com/dbuddha/alpine-gpui/issues/28)
- Requirements: [#37](https://github.com/dbuddha/alpine-gpui/issues/37), [#32](https://github.com/dbuddha/alpine-gpui/issues/32)
- Task: [#124](https://github.com/dbuddha/alpine-gpui/issues/124)
- Decision: [#137](https://github.com/dbuddha/alpine-gpui/issues/137)
- Research: [#118](https://github.com/dbuddha/alpine-gpui/issues/118)

## Motivation and boundary

Alpine owns a production AppKit window and bounded asynchronous presentation,
but the first Studio executable submits one fixed scene. This AEP adds the
smallest application boundary needed to turn native events and revisioned
background results into immutable latest-revision scenes. It does not add a
reactive graph, GPUI compatibility, a general async runtime, or multiple
windows.

## Atomic claims

- **AEP-0137-C01:** One `Application` owns one foreground delegate and a fixed
  number of standard worker threads. Request and result channels are bounded;
  foreground submission never waits for capacity; saturation and omitted
  results are counted.
- **AEP-0137-C02:** Every background request and result carries workspace,
  document, and process-local sequence identity. Only results matching both
  current revisions reach application state; stale results are counted and
  discarded before delegate mutation.
- **AEP-0137-C03:** Event mutation is synchronous on the main thread. A clean
  application builds no scene. A dirty application builds at most one immutable
  scene with the exact runtime-issued revision and current viewport.
- **AEP-0137-C04:** The safe native boundary preserves an owned `SurfaceEvent`
  vocabulary for keyboard, pointer, scroll, focus, resize, clipboard, IME,
  wake, and close without exposing AppKit objects. Production callbacks emit
  lifecycle, resize, focus, wake, and close; Task #130 owns complete native
  input and accessibility mapping.
- **AEP-0137-C05:** Close revokes foreground mutation and new frame production,
  closes worker admission, drains owned standard threads, and leaves native
  presentation teardown to the existing bounded surface lifecycle.

## Correctness and resource rules

Workspace and document revisions advance monotonically. Worker panic is caught
at the thread boundary and never unwinds through application or native code.
Result-channel saturation drops the result with evidence rather than blocking a
worker indefinitely. Returned scenes with the wrong revision or viewport are
rejected while dirty state remains pending.

The runtime owns no native handles. `AppContext` exposes invalidation, revision
advance, and nonblocking job submission. `WindowContext` exposes only the exact
scene revision and validated logical viewport. Worker completion wake is an
explicit replaceable callback; no timer, polling loop, or idle redraw is
introduced.

## Formal model and implementation mapping

[`RuntimeHandoff.tla`](../../formal/tla/aep-0137/RuntimeHandoff.tla)
models fixed request and result queue capacities, tagged job ownership,
revision advances, saturation, result omission, panic, current application,
stale rejection, and shutdown cancellation. `BoundedRequestQueue` and
`BoundedResultQueue` enforce the channel ceilings.
`CurrentApplicationIsCurrent` prevents stale foreground mutation, and
`ShutdownEventuallyDrains` supplies the bounded model's progress contract.

The deliberately faulty configuration applies one stale result as current and
must violate `CurrentApplicationIsCurrent`. `WorkerPool::submit` maps to
`Admit` or `RecordSaturation`; worker receive and terminal production map to
`Start`, `Complete`, `DropResult`, and `PanicJob`; foreground draining maps to
`ApplyCurrent` or `RejectStale`; runtime close maps to `BeginShutdown` and
`CancelOwned`. This is model checking of the stated abstraction, not a formal
refinement proof of Rust or standard-library channels.

## Accessibility, platform, performance, and memory

The event values preserve the identity needed by Task #130, but this AEP does
not claim native keyboard, IME, or accessibility qualification. Apple Silicon
macOS is the production event source; runtime state and worker behavior remain
host-testable through the safe unsupported-platform facade.

No latency superiority claim is introduced. Foreground submission and event
mutation contain no capacity wait, clean state creates no scene, and every
queue has fixed capacity plus current and peak accounting. Process footprint,
native wake latency, and sustained editor residency remain later qualification
work.

## Evidence and remaining risk

Unit controls cover saturation, stale-result rejection, worker panic, invalid
scene rejection, and clean-idle frame behavior. TLA+ covers the finite handoff
and shutdown abstraction. Native validation proves the complete event value
vocabulary crosses the retained delegate seam and that owner teardown remains
balanced. Hosted mutation, Miri, coverage, and `ci-pass` remain merge gates.

Real keyboard, pointer, scroll, clipboard, and IME conversion is deliberately
not inferred from replay evidence. Task #130 owns those callbacks, semantic
fixtures, accessibility synchronization, and input-latency qualification.

## Failure and reversal conditions

Worker creation and native surface errors remain structured. Queue saturation,
stale results, worker panic, invalid scenes, repeated close, and callback
reentrancy fail boundedly without panic. Revisit the standard-thread model only
if measured daily-driver workloads prove it dominant after correctness gates.
Do not introduce Tokio, detached work, unbounded channels, background state
mutation, or native handles in public runtime values.
