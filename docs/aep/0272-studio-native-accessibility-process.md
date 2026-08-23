# AEP 0272: Real Studio native accessibility process journey

- Status: Accepted
- Decision: [#268](https://github.com/dbuddha/alpine-gpui/issues/268)
- Research: [#267](https://github.com/dbuddha/alpine-gpui/issues/267)
- Implementation: [#272](https://github.com/dbuddha/alpine-gpui/issues/272)
- Requirement: [#37](https://github.com/dbuddha/alpine-gpui/issues/37)

## Motivation and journey

Alpine's portable semantic tests and AppKit fixture prove their respective
layers, but neither proved that the real Studio application composes them. The
required journey opens one bounded local workspace, expands its lazy tree,
opens two Rust files, navigates tabs, admits and activates one real product
diagnostic, edits, executes save through the native command row, rejects dirty
close, saves, closes, and drains all ownership.

## Goals

- Query the complete current native Studio tree through production AppKit
  selectors while returning no native handle.
- Select an exact current action target by semantic role and bounded label.
- Prove file-tree, tab, diagnostic, command, save, dirty-close, and close
  authorities compose through one real Studio process journey.
- Prove stable queries submit no frame and accepted visible actions submit at
  most one latest coalesced frame.
- Retain exact saved bytes and terminal native/runtime owner evidence.

## Non-goals

This AEP does not automate VoiceOver, claim external notification delivery,
replace Task #273's AX observer, measure physical latency or residency, add
another semantic tree, expose AppKit objects, or add a shipping test fixture,
dependency, plugin, AI, cloud, telemetry, GPUI, WGPU, or game-rendering path.

## Validation-only native boundary

The native inspector activates accessibility, reconciles the production cache,
and calls role, label, identifier, focus, selection, press-admission, validity,
and frame selectors on every current native element. It copies only bounded
strings, booleans, semantic IDs, and finite-frame evidence. The named action
probe requires exactly one current node matching role and label, invokes the
same `accessibilityPerformPress` implementation used by assistive technology,
and returns action and revocation evidence.

Both probes exist only under `alpine_native_validation`. They do not widen the
safe shipping surface, retain Studio state, create a callback registry, or
permit arbitrary actions.

Each probe performs one refresh. A redundant `accessibilityChildren` preflight
is prohibited because it doubles synchronous main-thread query work and allows
an asynchronous result to replace native instances between discovery and the
actual named operation.

The hosted process waits for each accepted action frame to reach a terminal
state before issuing the next action. This models successive AppKit run-loop
turns and prevents the test harness itself from exhausting the production
three-slot frame bound.

The process inherits the established presentation-evidence mode. Physical mode
uses compositor observations and may inject only the existing surface
configuration when AppKit has not yet published visibility. Explicit
`hosted-direct` mode injects one positive post-commit observation for each
expected frame because hosted CI may provide callback drawables without a
compositor presentation. The injected observation is armed before the run loop
turn, but the production presentation driver cannot terminalize it until Metal
command completion releases the frame slot. Invalid evidence-mode values fail
closed. Hosted mode proves Direct Metal commit, composition, terminal frame
ownership, and drain; it is not physical presentation, timing, AX client, or
VoiceOver evidence.

Document edits in this journey use `NSTextInputClient`'s production
`insertText:replacementRange:` selector and the live native responder epoch.
Synthetic `SurfaceEvent::Ime` values are prohibited here because an assumed
epoch can silently turn an intended edit into the correct stale-input no-op.
The probe installs dispatch before responder activation, publishes the exact
current focus state, and detaches without suspending the still-live input epoch.
Tab and diagnostic accessibility activation transfer semantic focus away from
the file tree just as their pointer equivalents do, so subsequent native text
targets the editor rather than being correctly ignored by a focused sidebar.

An accepted close revokes the native accessibility adapter before publishing
final focus loss. This prevents post-acceptance focus dispatch from requesting
a semantic refresh after runtime has already revoked new application work.
The `Allow` response also returns to AppKit without a semantic refresh, while a
cancelled close refreshes normally to publish its blocking status.

The production request bridge admits a frame only for an exact revision-valid
action whose typed result is `Applied` or `Unchanged`. It submits that frame
through the same bounded latest-scene helper used by keyboard and pointer input.
An `Unchanged` action may expose only dirty work that was already pending. Every
query, failed action, clipboard side effect, and close side effect with a frame
fails closed. This rule is consequential: the prior fixture never returned a
frame, so it hid that real visible Studio actions were being rejected
after mutation.

Runtime also separates query causality from concurrent worker publication. A
query may drain a newly completed bounded result so its semantic response is
current, but it does not consume the dirty scene. The next explicit wake emits
one latest coalesced frame. A portable regression queues a worker result directly
before a snapshot query and requires no query frame, retained dirty state, one
wake frame, then zero idle frames.

## Process composition and ownership

The custom native-process test starts an isolated qualification child. That
child receives an exact executable path to a shell wrapper which launches the
existing bounded mock Language Server Protocol fixture in a second process.
No process environment is mutated after threads exist, and no globally
installed language server is trusted.

After the complete journey succeeds, five isolated negative children each omit
exactly one required open, edit, accessibility action, save, or close step. A
control is accepted only when the same production journey rejects that omission;
an omitted step that still reaches qualification fails the parent process test.
Action totals are derived from successful native dispatches rather than encoded
as expected constants in the returned evidence.

Studio owns one real temporary Cargo workspace and uses its production runtime,
worker queues, file-tree actions, tab store, Rust diagnostic state, scene
builder, AppKit adapter, Direct Metal surface, atomic save, close response, and
teardown paths. The fixture directory is removed only after every owner drains.

## Correctness, performance, and memory

Every native query and action carries the revision returned by the current
Studio snapshot. Existing stale-action and input-epoch controls remain mandatory
companions. A stable query compares Metal submission counts before and after and
requires an exact zero delta. Each accepted visible action requires a delta no
greater than one. Worker progress is bounded to 1,024 explicit wake turns; no
timer poller or continuous frame loop is introduced.

The native tree cannot exceed the existing 271-node semantic ceiling. Evidence
copies one bounded role, label, and identifier per current node and is released
at the end of each synchronous probe. Close requires all ten native owner
classes inactive, exact acquisition/release parity for the nine exercised
classes, no pasteboard acquisition in this clipboard-free journey, no
release-order violation, no occupied frame slot, and the runtime shutdown
snapshot.

## Failure behavior

Missing, duplicate, disabled, revoked, or dispatch-failed native targets return
the existing structured accessibility validation failure. Worker or diagnostic timeout fails the
bounded journey. Save failure, dirty-close acceptance, unexpected bytes,
multiple query frames, more than one action frame, an active owner, or fixture
cleanup failure rejects qualification. The parent watchdog terminates a child
that exceeds fifteen seconds and retains stdout and stderr in its error.

## Formal applicability

This AEP adds no new production state machine. Input-epoch ordering remains
covered by AEP-0268, native identity and stale action rejection by AEP-0270, and
notification/revocation ordering by AEP-0271. The process journey is
implementation composition evidence and makes no TLA+ refinement claim.

## Atomic claims and evidence

- **AEP-0272-C01:** One real Studio workspace publishes bounded native roles,
  labels, focus, selection, identifiers, frames, and actions from its current
  semantic revision. Evidence: native child process, exact tree assertions,
  native mutation, and coverage.
- **AEP-0272-C02:** File rows, tabs, diagnostics, and command rows route through
  existing Studio authorities; stable queries submit zero frames and successful
  revision-valid actions submit at most one through the common frame helper.
  Evidence: native action sequence, frame deltas, keyboard companion journeys,
  rejected side-effect controls, and viable mutation rejection. This is a
  functional scheduling invariant, not a timing or renderer-performance claim.
- **AEP-0272-C03:** Editing, save, dirty-close rejection, second save, and close
  preserve exact fixture bytes and recovery authority. Evidence: filesystem
  assertions, production close delegate, and process watchdog controls.
- **AEP-0272-C04:** Language, worker, scene, frame, file, semantic, and native
  ownership is bounded and terminally drained. Evidence: bounded wake count,
  isolated server exit, runtime shutdown snapshot, frame evidence, and exact
  nine-class acquisition/release parity with all ten classes inactive.

## Risks and reversal conditions

Hosted AppKit remains API behavior, not physical VoiceOver evidence. Reopen if
Task #273's external observer cannot locate the same identifiers, physical
navigation requires a missing focus action, the deterministic fixture diverges
from the pinned real-server protocol, or the journey exposes positive residency
or frame-demand slope. Those findings require a superseding AEP rather than a
test-only product bypass.
