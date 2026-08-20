# AEP 0268: Bounded native input and accessibility lifecycle

- Status: Accepted
- Decision: [#268](https://github.com/dbuddha/alpine-gpui/issues/268)
- Research: [#267](https://github.com/dbuddha/alpine-gpui/issues/267)
- First implementation: [#269](https://github.com/dbuddha/alpine-gpui/issues/269)
- Requirement: [#37](https://github.com/dbuddha/alpine-gpui/issues/37)
- Parent tasks: [#130](https://github.com/dbuddha/alpine-gpui/issues/130), [#223](https://github.com/dbuddha/alpine-gpui/issues/223)

## Motivation and daily-driver journey

Alpine Studio already shapes and renders marked text for the editor, find,
quick open, command palette, and project search. Before this decision, those
owners trusted main-thread ordering alone. AppKit can discard an old conversion
session during focus or lifecycle changes and still deliver delayed callbacks.
Without an identity for the session, a delayed update or commit can mutate the
newly focused owner or document.

The daily-driver journey is: begin composition, lose focus or suspend, discard
native marked text, cancel exactly one Studio owner, invalidate the session,
publish focus loss, refocus under a distinct epoch, and reject every callback
that still names the old epoch. Accessibility then builds on the same bounded
main-thread lifecycle through Tasks #270 through #273.

## Goals

- Identify every IME value with one checked, non-zero, monotonic epoch.
- Make focus and lifecycle loss cancellation ordered and idempotent.
- Reject stale or prematurely future events before any application mutation.
- Cancel editor, find, quick open, command palette, and project search through
  one deterministic foreground path.
- Preserve dirty-only frame admission and bounded close-time drain.
- Separate portable, hosted AppKit, and trusted physical accessibility evidence.

## Non-goals

This decision does not add AccessKit, another semantic tree, a callback
registry, arbitrary text geometry, multi-window input, plugins, AI,
collaboration, cloud, telemetry, hosted VoiceOver automation, or a background
input worker. It does not claim that direct selector invocation proves physical
VoiceOver usability.

## Decision and state model

`InputEpoch` is an eight-byte handle-free value. Zero is invalid. The initial
surface epoch is one, and `checked_next` refuses representational wrap. Every
`SurfaceEvent::Ime` and `SurfaceEvent::Focus` carries the applicable epoch.

The native view has three logical states:

| State | Marked text | IME admission | Allowed transition |
| --- | --- | --- | --- |
| Focused(E) | absent or bounded | only E | suspend, close, update, commit |
| Composing(E) | bounded | only E | update, commit, cancel, suspend |
| Suspended(E+1) | absent | none | refocus as Focused(E+1), drain |

Suspension order is fixed: call `discardMarkedText`, suppress a reentrant
`unmarkText` commit, clear native marked text, emit one cancellation for E when
composition existed, advance to E+1, mark input inactive, then publish focus
loss carrying E+1. Repeated suspension is an idempotent no-op. Refocus activates
the already advanced epoch and publishes it once.

Studio classifies an IME epoch as current, stale, or future. Only current input
while focused reaches an owner. Stale and future values update independent
saturating counters and do not mutate buffers, selections, queries, workers,
focus, or frame demand.

No TLA+ refinement claim is made. This finite transition is mapped directly to
the Rust value contract, Kani boundary proof, portable owner fixtures, and the
production AppKit replay. A later cross-thread input design would require a new
model and superseding decision.

## Rust and native ownership

`alpine-platform-macos` owns `InputEpoch`, its admission vocabulary, and the
event fields on every target. `SurfaceView` owns the current epoch, active bit,
marked text, discard reentrancy guard, and rejected-callback count. All native
state remains main-thread-only and private; no AppKit handle crosses the public
contract.

`StudioApp` owns the expected epoch and stale/future evidence counters. It
remains the sole owner of editor and transient-surface composition state.
Cancellation does not clone text, retain a callback, or enqueue work.

## Correctness and failure behavior

- Epoch exhaustion fails closed: input becomes inactive and dispatch records a
  structured native failure rather than reusing an identity.
- Handler absence or close still clears native marked text and deactivates input.
- Reentrant AppKit discard cannot become a commit because `unmarkText` observes
  the discard guard.
- A stale focus event cannot roll Studio back to an obsolete epoch.
- A future IME event cannot establish an epoch; only a focus transition can.
- Duplicate loss and cancellation do not schedule another frame.

## Accessibility consequences

Task #269 establishes the focus and IME prerequisite for native accessibility.
Task #270 adds bounded actions, identity, focus, and rectangles. Task #271 adds
notification payload and destruction semantics. Task #272 proves the real
Studio process journey. Task #273 separately captures trusted `AXObserver`,
Accessibility Inspector, and human VoiceOver evidence. These tasks reuse one
Studio semantic model and may not widen this input ownership contract silently.

## Performance and memory

The hot-path cost is one eight-byte event field, two scalar comparisons, and a
predictable branch before IME mutation. Native and Studio each retain one epoch
plus fixed scalar counters. No allocation, worker, channel, timer, polling,
Metal wait, filesystem operation, or continuous redraw is introduced.
Cancellation requests a frame only when visible composition or focus changed.

## Atomic claims and evidence mapping

- **AEP-0268-C01:** Native suspension discards marked text, emits at most one old
  epoch cancellation, advances without wrap, and publishes loss in that order.
  Evidence: production AppKit replay, reentrancy guard controls, Kani boundary
  proof, viable mutation rejection, Miri, and hosted native validation.
- **AEP-0268-C02:** Stale and future IME events are atomic no-ops before every
  Studio mutation authority. Evidence: portable owner fixtures, buffer revision
  controls, stale/future counters, coverage, and mutation.
- **AEP-0268-C03:** Editor, find, quick open, command palette, and project search
  each cancel through the same focus transition, and duplicate loss is
  idempotent. Evidence: one fixture per owner and native loss/refocus replay.
- **AEP-0268-C04:** Close, occlusion, minimization, and handler revocation drain
  fixed native state without idle redraw or external waits. Evidence: native
  lifecycle tests, close fault injection, post-close accounting, and soak.
- **AEP-0268-C05:** Accessibility operation and external-delivery claims remain
  separated across hosted and trusted physical lanes. Evidence: Tasks #270
  through #273 and final qualification #253.

## Risks and reversal conditions

The principal risk is an AppKit callback path that cannot be associated with
the current input context after refocus. Reopen this decision if physical input
methods demonstrate such ambiguity, if epoch exhaustion needs recoverable
surface replacement, if bounded element rectangles are insufficient for
VoiceOver navigation, or if external AX evidence disproves the minimal action
and focus contract. Any second platform must preserve semantic ordering without
copying AppKit-specific mechanisms into shared Studio state.
