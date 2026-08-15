# AEP 0064 native presentation lifecycle model

`PresentationLifecycle.tla` models one main-thread-owned macOS surface, one
`CAMetalDisplayLink`, one active frame attempt, bounded invalidations, bounded
surface changes, and shutdown. The model distinguishes eligibility at commit
from qualification after completion. A frame that was current when submitted
can become superseded while in flight, but it cannot qualify as the current
presented revision.

- `app` maps to application admission and native owner teardown.
- `link` maps to paused, running, and invalidated `CAMetalDisplayLink` states.
- `visible`, `sized`, and `dirty` map to native presentation eligibility.
- `requestedRevision` and `presentedRevision` map to coalesced application and
  accepted presentation revisions.
- `surfaceEpoch` maps to resize, backing-scale, and display identity changes.
- `phase` maps to preparation, callback encoding, command submission, the
  direct presentation call, and the terminal return to idle.
- `resource` maps to no drawable, an exclusively owned callback drawable, and
  the in-flight drawable boundary.
- `attemptSubmits`, `presentCalls`, and `eligibleAtSubmit` map to per-attempt
  instrumentation.
- `outcome` maps to distinct presented, superseded, cancelled, and failed evidence.

The model actions map to planned pure Rust transitions. `BeginUpdate` maps to
the `CAMetalDisplayLinkDelegate` callback and exclusive receipt of its drawable.
`Submit` maps to one command-buffer commit after the epoch and revision check.
`CallPresent` maps to the required direct drawable presentation call.
`CompletePresentation` maps to later terminal correlation.
`AdvanceSurfaceEpoch`, `ToggleVisibility`, and `ToggleSize` model
native events that can supersede prepared or submitted work. The shutdown
actions map to display-link invalidation, rejection of new work, drawable
release, in-flight drain, and final native teardown.
`CancelActive` maps to explicit Rust cancellation and never aliases a stale
attempt or execution failure. A committed attempt can cancel only after
`BeginShutdownCommitted` places the owner in its draining state.
An idle dirty request cancelled before `Prepare` has no attempt identity; Rust
records its requested revision and surface epoch separately rather than
fabricating commit or presentation evidence.

The pull-request model bounds revisions at two, surface epochs at one, and
combined visibility, size, or epoch changes at one. The nightly model expands
those bounds to three revisions, two epochs, and two environment changes. The
bounded environment-change counter prevents an adversarial display from
toggling forever and makes progress assumptions explicit. Weak fairness is
applied to continuously enabled framework actions. It does not assume that a
temporarily hidden or zero-sized surface must present.

The model excludes actual AppKit and Objective-C thread enforcement, callback
delivery timing, GPU execution, drawable timeout duration, Core Animation
scanout, pixels, colorspace, input, accessibility, multi-window interaction,
device recovery, unbounded event streams, elapsed time, energy, and memory. It
does not prove that the Rust implementation refines this model.

Conformance requires pure Rust transition tests that replay model-derived
traces, Kani harnesses for bounded revisions and epochs, native tests for the
AppKit and Metal boundary, offscreen semantic and pixel equivalence, lifecycle
failure injection, and fixed-hardware measurements. A committed drawable can
become stale after submission; Alpine records it as superseded and never uses
it as evidence for the current surface state. This is not a claim that Core
Animation can retract work already committed to the display system.

`Faulty.cfg` enables `FaultyPresentStale`, which marks an older revision or
surface epoch as the current presented result. TLC must report a
`PresentedIsCurrent` invariant violation.
