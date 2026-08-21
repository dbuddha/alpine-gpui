# AEP 0271: Bounded accessibility notifications and destruction

- Status: Accepted
- Decision: [#268](https://github.com/dbuddha/alpine-gpui/issues/268)
- Research: [#267](https://github.com/dbuddha/alpine-gpui/issues/267)
- Implementation: [#271](https://github.com/dbuddha/alpine-gpui/issues/271)
- Requirement: [#37](https://github.com/dbuddha/alpine-gpui/issues/37)

## Motivation and journey

Alpine already publishes one bounded semantic tree and revision-safe native
actions. A daily-driver editor must also tell AppKit when current layout or
announcing status changes and when an obsolete native element is destroyed. The
journey is: reconcile one snapshot, remove obsolete identities from the current
cache, release the adapter borrow, post destruction before current-state
notifications, and release every temporary native owner before returning.

## Goals

- Post layout notifications with `NSAccessibilityUIElementsKey` naming only
  bounded elements from the current snapshot.
- Post announcements with bounded text, `NSAccessibilityAnnouncementKey`, and
  medium `NSAccessibilityPriorityKey`.
- Post destruction for removed, semantically replaced, and revoked instances.
- Reject late selectors and actions before destruction is posted.
- Revoke the application handler only after close-time destruction drains, then
  permit no later post.
- Account for post calls, payload bytes, temporary retained slots, omissions,
  invalid user-info, and post-after-revocation violations.

## Non-goals

This AEP does not claim notification receipt, spoken output, braille usability,
VoiceOver automation, arbitrary text geometry, another semantic tree, AccessKit,
background dispatch, polling, continuous rendering, or GPU work. Task #273 owns
physical AXObserver and human VoiceOver evidence.

## Contract and ordering model

The post order for one refresh is `Retire -> InstallCurrent -> ReleaseBorrow ->
Destroyed* -> Layout? -> Focus? -> Selection? -> Value? -> Announcement* ->
ReleaseBatch`. Layout payloads always include the current root and any current
node whose structural or bounded layout semantics changed. Removed elements are
never named by the layout payload.

Close uses `InvalidateIdentity -> MoveCurrentToBatch -> ReleaseBorrow ->
Destroyed* -> ReleaseBatch -> RevokeHandler`. Reentrant queries observe no
current instance while close posts are in progress. A repeated or reentrant
revoke is a no-op. No native notification is admitted after handler revocation.

The finite model in `formal/tla/aep-0271` checks obsolete-instance separation,
post-borrow dispatch, destruction-before-ordinary ordering, bounded ownership,
post-revocation exclusion, and closed drain. Three faulty controls must expose
post-under-borrow, early ordinary posting, and early revocation defects. No Rust
refinement claim is made.

## Rust and native ownership

The private AppKit adapter remains the only native owner. A refresh outcome owns
only retained AppKit element slots and one shared bounded announcement name.
Semantic names already satisfy the 4 KiB per-node and 256 KiB aggregate limits.
Layout payloads contain at most the existing 271 current elements. Native
dictionaries are constructed and posted only after `RefCell` borrows end.

Semantic replacement is counted as both one created and one retired native
instance even when the numeric semantic ID is unchanged. The old instance fails
the current generation and instance check before its destruction post. Close
moves every current entry out of the cache before posting, so a reentrant query
cannot revive it.

## Correctness, performance, and memory

The batch has a fixed semantic ceiling and no persistent queue. Exact retained
accounting includes each `Retained` slot plus announcement UTF-8 bytes; native
framework object overhead remains outside Alpine-owned byte claims. Posting is
synchronous on the main thread because that is AppKit's contract, but it does
not wait on Metal, a worker, a channel, a timer, or filesystem work. It never
requests a frame.

Counters prove completed AppKit post invocation only. Positive and malformed
payload controls independently require each AppKit dictionary key. A repeated
revoke proves the transition is admitted once and ends with no handler or
revocation in progress. The bounded protocol
fixture records kind, stable target identity, payload element count, payload
bytes, and priority for comparison with Task #273's external observer. Neither
counter nor fixture claims external delivery.

## Failure behavior

Snapshot, intent, or layout-payload allocation failure returns the existing
structured driver error before publication. Missing current targets omit the
corresponding current-state notification. Invalid or revoked elements reject
late selectors and actions. Repeated revoke does not post twice. Saturating
diagnostic counters cannot affect admission or ownership.

## Atomic claims and evidence

- **AEP-0271-C01:** Layout and announcement posts contain the exact required
  bounded AppKit user-info keys and values. Evidence: production payload
  construction, positive and malformed-key controls, protocol records, native
  mutation, and coverage.
- **AEP-0271-C02:** Removed or replaced instances leave the current identity set
  before destruction, and ordinary notifications follow destruction outside the
  adapter borrow. Evidence: TLA+ invariants, faulty controls, and native replay.
- **AEP-0271-C03:** Close destroys every remaining instance before handler
  revocation, rejects late access, posts nothing later, and drains ownership.
  Evidence: TLA+ revocation control, repeated native revoke, terminal-state
  evidence, native fault path, and owner accounting.
- **AEP-0271-C04:** Notification work is bounded, byte-accounted, dirty-neutral,
  and independent of frame, GPU, worker, and filesystem paths. Evidence: native
  counters, frame controls, retained-byte controls, mutation, and coverage.

## Risks and reversal conditions

Hosted AppKit proves invocation but not receipt. Reopen if physical AXObserver
evidence requires different coalescing, priority, target, or ordering; if AppKit
reentrancy reveals another lifetime state; or if measured accessibility-on
residency exceeds the bounded contract. Such findings require a superseding AEP,
not an untracked compatibility layer.
