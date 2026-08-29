# AEP 0120: Bounded asynchronous Metal presentation

- Status: accepted 2026-08-16
- Capability: [#28](https://github.com/dbuddha/alpine-gpui/issues/28)
- Requirement: [#37](https://github.com/dbuddha/alpine-gpui/issues/37)
- Task: [#123](https://github.com/dbuddha/alpine-gpui/issues/123)
- Decision: [#120](https://github.com/dbuddha/alpine-gpui/issues/120)
- Research: [#118](https://github.com/dbuddha/alpine-gpui/issues/118), [#113](https://github.com/dbuddha/alpine-gpui/issues/113)
- Supersedes: the synchronous callback-completion portion of AEP 0064

## Motivation and journey

The first native surface commits, directly presents, and then calls
`waitUntilCompleted` inside the main-run-loop display callback. That preserves
simple terminal ownership, but serializes callback return with GPU completion
and cannot sustain text-heavy high-refresh editing.

Alpine replaces that wait with exactly three reusable frame-resource slots.
Each callback either acquires one free slot, commits and directly presents once,
then returns, or records saturation and leaves only the newest revision dirty.
Metal completion may occur on a native completion thread, but only an owned
terminal record crosses that boundary. Generation, slot sequence, frame token,
revision, and surface epoch are checked on the main thread before publication.

## Goals and non-goals

Goals are non-blocking display callbacks, three-slot ownership, ABA-safe slot
reuse, bounded completion records, explicit saturation, current-only
publication, completion reordering, deterministic shutdown drain, reusable
geometrically grown upload buffers, exact byte accounting, and zero clean-idle
pacing after all slots terminate.

This AEP does not add a general async runtime, background scene construction,
more than three slots, multi-window scheduling, text primitives, optical
latency, or a performance superiority claim. Offscreen readback remains
synchronous because CPU pixel ownership requires terminal GPU completion.

## Atomic claims

- **AEP-0120-C01:** Exactly three allocation-free portable slot identities own
  encoding or committed work. Admission never exceeds three; saturation is an
  observable bounded omission; every release balances one unique admission.
- **AEP-0120-C02:** Every lease carries slot, monotonic sequence, owner
  generation, and frame token. Completion can release stale work, but only a
  successful result matching current generation, revision, and epoch can be
  classified for current publication. Reordered or ABA-stale completion cannot
  release a replacement lease.
- **AEP-0120-C03:** Native presentation commits and directly presents inside
  the display callback, installs one completion handler, and returns without a
  GPU wait. Completion threads publish owned terminal data only; main-thread
  lifecycle code performs all surface mutation and qualification.
- **AEP-0120-C04:** Each slot reuses one shared upload buffer. Capacity grows to
  the next power of two under an 8 MiB per-slot hard limit, records current and
  peak bytes, and trims oversized capacity after explicit pressure or 120
  terminal reuse opportunities. The three-slot upload ceiling is 24 MiB.
- **AEP-0120-C05:** Close revokes callback admission first, cancels precommit
  slots immediately, and drains committed slots. Stale-generation callbacks
  can release resources but cannot publish success or resume pacing.
- **AEP-0120-C06:** Handle-free evidence separates occupied and submitted
  slots, peak occupancy, saturation, upload capacity, allocation, retained
  bytes, encode and commit, GPU completion, direct present, compositor
  presentation, terminal status, and omission reason.

Claims C02 through C06 enter the evidence registry only when their complete
integration evidence exists. C01 begins with the portable slot model; that
model is not evidence that the callback wait has been removed.

## Formal and executable model

[`FrameSlots.tla`](../../formal/tla/aep-0120/FrameSlots.tla) models three fixed
slots, free, encoding, and submitted phases, monotonic lease sequences, owner
generation, revision and epoch supersession, completion reorder, cancellation,
a one-bit saturation witness, and shutdown drain. Repeated saturation counts are
stutter-equivalent in this model and remain exact in the compiled Rust evidence.
Inactive publication identity is canonicalized because no transition or checked
property consumes it. `BoundedFrameSlots`, `BalancedOwnership`,
`OwnedSequencesUnique`, and `InactivePublicationHasNoIdentity` constrain
ownership and the quotient. `CurrentPublicationIsCurrent` rejects stale
publication. `ShutdownEventuallyDrains` is a progress property.

The faulty configuration deliberately publishes one stale submitted slot as
current. TLC must report a `CurrentPublicationIsCurrent` violation. The model
excludes native APIs, elapsed time, buffer bytes, Objective-C retain counts,
pixels, and Rust refinement.

`FrameSlotRing` is the corresponding allocation-free Rust model. It returns an
opaque lease and treats saturation as an admitted observable outcome rather
than an unbounded queue. Invalid phase and lease transitions restore exact prior
state. Unit, bounded-sequence, public integration, and Kani evidence exercise
the compiled implementation.

## Native ownership and scheduling

`alpine-metal` owns three private native slot records. A submitted record
retains its command buffer, reusable upload buffer, completion signal, frame
identity, and accounting until terminal completion. `alpine-platform-macos`
owns only an opaque submission identity aligned with the portable lease and its
callback drawable until presentation correlation is terminal. Applications and
portable scenes see no native handle.

The display link remains active while current dirty work or submitted slots
need main-thread correlation. It pauses when neither exists. If all slots are
occupied, no fourth command buffer is created; the newest requested revision
remains coalesced for a later callback. Completion order does not determine
publication order.

## Correctness, performance, and memory

Correctness gates timing. Every completed command must retain a structured
status and copied native error before resources release. Device loss
invalidates its backend generation. A compositor presentation timestamp and a
GPU completion are distinct evidence; neither is optical latency.

The 8 MiB per-slot upload limit admits 262,144 current 32-byte quad instances,
four times the existing 65,536-operation comparator bound. Larger presentation
uploads fail before native allocation. Future glyph or path storage requires a
separate budget rather than silently consuming this one.

Hosted timing is informational. Fixed-hardware claims require the comparator
protocol, semantic admission, stage-separated distributions, byte and physical
footprint evidence, and calibrated independent windows.

## Failure and reversal conditions

Missing drawable, saturation, allocation failure, encode failure, close during
encode, close after commit, delayed or reordered completion, resize, display
change, device interruption, command failure, and shutdown are explicit
outcomes. Unsupported operations never panic.

Revisit exactly three slots or the upload budget only if Apple contracts change
or fixed-hardware correctness, latency, and residency evidence proves another
bounded design superior. Do not replace completion handlers with callback waits,
application polling, unbounded queues, or a general async runtime.
