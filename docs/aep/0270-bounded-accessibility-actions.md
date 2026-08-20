# AEP 0270: Bounded accessibility actions and geometry

- Status: Accepted
- Decision: [#268](https://github.com/dbuddha/alpine-gpui/issues/268)
- Research: [#267](https://github.com/dbuddha/alpine-gpui/issues/267)
- Implementation: [#270](https://github.com/dbuddha/alpine-gpui/issues/270)
- Requirement: [#37](https://github.com/dbuddha/alpine-gpui/issues/37)

## Motivation and journey

Alpine already exposes one bounded, revisioned semantic tree and exact UTF-16
text queries. A daily-driver editor also needs assistive technology to locate
semantic elements and invoke the same commands used by keyboard and pointer
input. The journey is: AppKit reads one current node, obtains its stable external
identifier and screen rectangle, invokes press, and Alpine revalidates the node
and document/buffer revision before the existing Studio command mutates state.

## Goals

- Publish finite, bounded view-local rectangles and convert them at the AppKit boundary.
- Preserve one current focused node and stable external identifiers.
- Add one closed `Activate` action for current enabled nodes.
- Route tabs, visible file rows, command results, visible diagnostics, save, and
  dirty-close behavior through existing Studio authorities.
- Reject stale, missing, disabled, oversized, and revoked requests before mutation.
- Keep queries dirty-neutral and accepted actions within one coalesced frame.

## Non-goals

This AEP does not add `SetFocus`, arbitrary text-range geometry, a callback
registry, a second semantic tree, AccessKit, plugins, background accessibility
work, continuous rendering, or physical VoiceOver qualification. Tasks #271
through #273 retain notification, process, and trusted physical evidence.

## Contract and model

`AccessibilityBounds` stores four normalized finite `f32` bit patterns. Each
coordinate and extent, and each checked axis sum, is at most 1,048,576 view
points. A node states whether it supports activation and whether that action is
currently enabled. `Activate` carries the exact document/buffer revision and
semantic node identity observed by AppKit.

The transition is `Observed -> ValidateRevision -> ResolveNode -> ValidateAction
-> ExistingCommand -> Applied|Unchanged`. Every failed edge terminates before
Studio mutation. No asynchronous action queue exists.

The native adapter keys cached elements by semantic identity, surface
generation, and a monotonic element-instance generation. An unchanged semantic
node preserves its native object and external identifier. Removal or semantic
replacement releases that instance; a retained obsolete object cannot become
valid when a numeric semantic slot is reused. Close revokes the surface
generation and all remaining instances.

## Rust and native ownership

`alpine-platform-macos` owns bounds, action vocabulary, validation, and
handle-free accounting. Studio derives nodes directly from tabs, visible file
rows, command matches, visible diagnostics, focus, and immutable text state.
The private AppKit adapter owns weak view references and native element caches.
No Studio object, callback, AppKit handle, or document text enters a node.

AppKit receives `accessibilityIdentifier`, `accessibilityFrame`, and
`accessibilityPerformPress`. Bounds remain view-local in the safe contract and
become screen coordinates only through the current window at the native edge.
Press is admitted only when the current node supports activation and remains
enabled.

## Correctness, performance, and memory

Snapshot validation retains the existing 271-node and name-byte ceilings. Node
accounting includes the added bounds and action scalars through exact struct
size. File, command, and diagnostic children are limited to already bounded
visible projections. Queries allocate no application state, schedule no worker,
and request no frame. Activation performs one bounded snapshot validation and
then calls one existing foreground command; its resulting visual effect enters
the existing latest-wins frame coalescer.

External identifiers are formatted on demand and are not retained in the
semantic snapshot. Native cache residency remains bounded by the node ceiling,
and revocation releases every retained element. This slice adds no GPU work,
idle rendering, timer, channel, filesystem authority, or general element layer.

## Failure behavior

Invalid bounds fail construction. Exact revision mismatch returns
`StaleRevision`; absent nodes return `ActionTargetMissing`; unsupported or
disabled nodes return `ActionDisabled`. Native handler absence, stale instance,
surface revocation, request mismatch, or response mismatch returns false to
AppKit and cannot dispatch a command. Dirty close continues through Studio's
existing fail-closed close policy and cannot discard text.

## Atomic claims and evidence

- **AEP-0270-C01:** Bounds and activation values are finite, bounded, exact, and
  handle-free. Evidence: public value tests, checked mapping proof, accounting,
  Miri, and mutation.
- **AEP-0270-C02:** Studio resolves one exact current node and routes only the
  approved existing command authorities; stale, missing, and disabled actions
  are no-ops before mutation. Evidence: semantic journeys and runtime frame controls.
- **AEP-0270-C03:** Unchanged native elements preserve external identity, replaced
  or revoked instances never revive, and view-local bounds become screen frames.
  Evidence: production AppKit selector replay and close-time accounting.
- **AEP-0270-C04:** Queries remain dirty-neutral and one accepted action requests
  at most one coalesced frame without unbounded retained state. Evidence: runtime
  integration, native counters, coverage, and mutation.

## Risks and reversal conditions

Screen conversion must be checked on physical display migration in Task #273.
Reopen this decision if physical VoiceOver requires explicit `SetFocus`, if a
supported control cannot be expressed as the closed action, if semantic slot
replacement can revive an old native object, or if visible projection fails the
daily-driver navigation journey. A future general Alpine element layer may
produce these same nodes, but it may not add a second accessibility tree.
