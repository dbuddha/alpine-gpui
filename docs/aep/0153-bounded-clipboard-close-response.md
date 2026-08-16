# AEP 0153: Bounded clipboard and close responses

- Status: accepted 2026-08-16
- Capability: [#28](https://github.com/dbuddha/alpine-gpui/issues/28)
- Requirement: [#37](https://github.com/dbuddha/alpine-gpui/issues/37)
- Tasks: [#154](https://github.com/dbuddha/alpine-gpui/issues/154), [#130](https://github.com/dbuddha/alpine-gpui/issues/130), [#126](https://github.com/dbuddha/alpine-gpui/issues/126)
- Decision: [#153](https://github.com/dbuddha/alpine-gpui/issues/153)
- Research: [#118](https://github.com/dbuddha/alpine-gpui/issues/118)

## Motivation and journey

The one-file editor can receive keyboard and IME events and save a local file,
but its native callback returns only an optional frame. That result cannot
return selected text for copy or cut and cannot synchronously distinguish an
accepted close from a data-loss veto. Passing AppKit objects into the runtime
would violate Alpine's safe, handle-free platform boundary.

The selected journey is one bounded synchronous event response. A foreground
delegate may return at most one immutable frame, at most one validated
plain-text clipboard write, and one close disposition. Ordinary events carry no
close request. An allowed close revokes runtime admission; a cancelled close
keeps the same application and window live.

## Goals and non-goals

The portable contract owns validated UTF-8 text, rejects paste as a write
identity, preserves copy or cut identity, prevents a second clipboard write in
one event, and makes close cancellation explicit. The existing frame-only
dispatch remains available while native integration is developed.

This AEP does not authorize rich clipboard formats, clipboard history, native
handles in public values, prompts, autosave, save-as, recovery journals,
multiple windows, plugins, telemetry, network access, or background clipboard
polling.

## Atomic claims

- **AEP-0153-C01:** `ClipboardText` retains no more than 64 MiB of owned UTF-8,
  and one event response contains at most one clipboard write.
- **AEP-0153-C02:** `SurfaceResponse` keeps frame, clipboard, and close evidence
  independent. Frame-only dispatch discards side effects explicitly rather
  than changing its return contract.
- **AEP-0153-C03:** A close request starts as allowed. A delegate may cancel it
  exactly while that request is active. Allowed close revokes foreground work
  and dirty-frame production; cancelled close keeps both live.
- **AEP-0153-C04:** Native copy and cut publish typed completion only after the
  bounded AppKit write terminates. Native paste checks its UTF-8 byte length
  before constructing Alpine-owned text and publishes success or structured
  failure without retaining an AppKit object.
- **AEP-0153-C05:** `windowShouldClose` admits irreversible close only after a
  synchronous `Allow` response. Cancel, missing response, reentrant dispatch,
  and dispatch failure veto close; `windowWillClose` performs drain only.

Studio selection/revision correlation, post-success cut mutation, and visible
local failure status remain required by Task #154 and are not claimed by this
native transport slice.

## Model and implementation mapping

[`ClipboardCloseResponse.tla`](../../formal/tla/aep-0153/ClipboardCloseResponse.tla)
models one bounded response slot and the requested, cancelled, and allowed close
states. `ClipboardResponseIsBounded` maps to `ClipboardText::new`,
`AppContext::write_clipboard`, and `SurfaceResponse`. `CancelledCloseStaysLive`
and `AllowedCloseRevokesAdmission` map to
`Application::dispatch_with_response`. The faulty configuration closes after a
cancel and must violate `CancelledCloseStaysLive`.

The model does not refine Rust allocation, AppKit pasteboard behavior, native
callback reentrancy, or operating-system close delivery. Native validation
tests those concrete boundaries independently and makes no formal-refinement
claim.

## Ownership and correctness

Clipboard text is one `Box<str>` with no replica, platform object, or hidden
history. `ClipboardWrite` consumes validated text and accepts only copy or cut.
`AppContext` owns borrowed event-local response slots and rejects duplicate
writes or cancellation outside an allowed close. Worker-result contexts expose
neither response capability.

`Application::dispatch_with_response` drains current worker results, applies
one synchronous event, then resolves close before building a frame. An allowed
close clears dirty state and returns no frame. A cancelled close may still
produce one latest dirty frame. Calls after shutdown return an empty response.

The frame-only `Application::dispatch` is a compatibility path for callers
that intentionally have no native side effects. Task #154 native integration
must call `dispatch_with_response`; using frame-only dispatch there would
silently discard clipboard writes and close dispositions and invalidates the
native acceptance evidence.

## Accessibility, performance, and memory

No accessibility claim is introduced. The later native slice must preserve
keyboard-only copy, cut, paste, and close behavior and expose failure status to
assistive technology.

The portable path adds no thread, channel, timer, polling, or idle redraw. One
response contains three fixed cardinality slots. Retained clipboard text is
bounded to 64 MiB; the current tests do not claim native transient allocation,
process footprint, or clipboard latency.

## Platform scope and evidence

The values and runtime transition are portable safe Rust. Unit tests prove
constructor bounds, operation identity, independent response channels,
duplicate rejection, cancelled-close liveness, and allowed-close shutdown.
TLA+ checks the corresponding finite state abstraction and a deliberately
faulty cancel transition.

Apple Silicon native validation exercises copy, cut, paste, injected write
failure, bounded owned completion values, synchronous cancel, synchronous
allow, and drain ordering through the production delegate methods. It uses a
unique validation pasteboard to avoid changing a developer clipboard; shipping
uses `generalPasteboard` through the same read and write conversion functions.
Studio mutation, visible status, and full process journeys remain unqualified.
No product or performance claim may cite this AEP alone.

## Risks and reversal conditions

The 64 MiB ceiling may be too high or low for measured daily-driver workloads.
Change it only with explicit degraded behavior and memory evidence. Revisit the
synchronous response if measured pasteboard or close callbacks violate the
input gate, but preserve bounded ownership, two-phase cut safety, structured
failure, and deterministic close veto.
