# AEP 0137: Bounded single-window Studio runtime

- Status: accepted 2026-08-15
- Capability: [#28](https://github.com/dbuddha/alpine-gpui/issues/28)
- Requirements: [#37](https://github.com/dbuddha/alpine-gpui/issues/37), [#32](https://github.com/dbuddha/alpine-gpui/issues/32), [#34](https://github.com/dbuddha/alpine-gpui/issues/34)
- Tasks: [#124](https://github.com/dbuddha/alpine-gpui/issues/124), [#210](https://github.com/dbuddha/alpine-gpui/issues/210), [#211](https://github.com/dbuddha/alpine-gpui/issues/211), [#215](https://github.com/dbuddha/alpine-gpui/issues/215), [#389](https://github.com/dbuddha/alpine-gpui/issues/389)
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
  foreground submission never waits for capacity; request saturation is
  counted. A completed standard worker waits at the bounded result channel
  instead of omitting accepted work; foreground disconnection is counted.
  Pending standard-worker completions are bounded by result capacity plus one
  owned completion per fixed worker.
  Independent local producers use a separate fixed result queue with item,
  retained-byte, wake, rejection, and drain accounting.
- **AEP-0137-C02:** Standard worker requests and results carry workspace,
  document, and process-local sequence identity; only results matching both
  current revisions reach application state. Independent producer payloads
  carry application-owned identity and cross the same main-thread delegate
  boundary without submit-time revision rejection, so the delegate can admit
  the exact current payload before mutation.
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
Result-channel saturation backpressures only a standard background worker and
never omits an accepted completion. Shutdown disconnects the result receiver
before joining workers so blocked publication terminates without deadlock;
completion omitted after disconnection is counted. External result admission
serializes only the nonblocking channel operation, rejects capacity or shutdown
structurally, and never exposes native handles. Returned scenes with the wrong
revision or viewport are rejected while dirty state remains pending.

The runtime owns no native handles. `AppContext` exposes invalidation, revision
advance, nonblocking job submission, and a cloneable `ExternalProducer` whose
owned payload type is the delegate's existing worker-output type. `WindowContext`
exposes only the exact scene revision and validated logical viewport. Worker and
external completion wake use one explicit replaceable callback; empty-to-nonempty
external admission coalesces wake requests, and no timer, polling loop, or idle
redraw is introduced.

## Formal model and implementation mapping

[`RuntimeHandoff.tla`](../../formal/tla/aep-0137/RuntimeHandoff.tla)
models fixed request and result queue capacities, tagged job ownership,
revision advances, request saturation, backpressured completion, panic, current
application, stale rejection, and shutdown cancellation. `BoundedRequestQueue`
and `BoundedResultQueue` enforce the channel ceilings;
`BoundedWorkers` and `BoundedCompletionOwnership` constrain retained completion
ownership across workers and the result queue.
`CurrentApplicationIsCurrent` prevents stale foreground mutation, and
`RunningEventuallyResolves` and `ShutdownEventuallyDrains` supply the bounded
model's completion and shutdown progress contracts. Formal saturation and
application-result counts are one-bit witnesses because their exact history
does not influence a modeled guard or checked property; compiled runtime tests
retain exact counter coverage. Applied, stale-rejected, panicked, and cancelled
jobs share one canonical terminal storage state after ownership ends; their
distinct actions and current or stale occurrence witnesses preserve the modeled
outcomes. Terminal jobs clear revision identity after it becomes unobservable,
with `TerminalJobsHaveNoTag` enforcing that quotient. A direct
`applicationIsCurrent` monitor remains true for the exact-tagged apply action
and becomes false only for the deliberate stale-apply control, so the safety
invariant does not retain historical result identity in unrelated states.

The deliberately faulty configuration applies one stale result as current and
must violate `CurrentApplicationIsCurrent`. `WorkerPool::submit` maps to
`Admit` or `RecordSaturation`; worker receive and terminal production map to
`Start`, capacity-gated `Complete`, and `PanicJob`; foreground draining maps to
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
queue has fixed capacity plus current and peak accounting. Independent payloads
are capped at 16 queued items and 8 MiB of attributed retained bytes. Process
footprint, native wake latency, and sustained editor residency remain later
qualification work.

## Evidence and remaining risk

Unit controls cover worker and external saturation, external wake coalescing,
cross-revision external delivery, stale worker rejection, retained-byte release,
result backpressure, shutdown disconnection, shutdown revocation, worker panic,
invalid scene rejection, and clean-idle frame behavior. Studio's language
composition additionally preserves one latest process-wake generation across
temporary shared-queue saturation, admits diagnostics through exact application
identity, and leaves stale or unchanged language output non-invalidating. TLA+
covers the revision-stamped worker handoff, capacity-gated completion, and
shutdown abstraction; it does not claim the later independent producer
extension. Native validation proves the complete event value vocabulary crosses
the retained
delegate seam and that owner teardown remains balanced. Hosted mutation, Miri,
coverage, and `ci-pass` remain merge gates.

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
