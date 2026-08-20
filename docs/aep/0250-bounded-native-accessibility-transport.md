# AEP 0250: Bounded native accessibility transport

- Status: Accepted
- Decision: [#250](https://github.com/dbuddha/alpine-gpui/issues/250)
- Task: [#251](https://github.com/dbuddha/alpine-gpui/issues/251)
- Requirement: [#37](https://github.com/dbuddha/alpine-gpui/issues/37)
- Parent tasks: [#130](https://github.com/dbuddha/alpine-gpui/issues/130), [#223](https://github.com/dbuddha/alpine-gpui/issues/223)

## Context

Studio already owns a bounded revisioned semantic tree and checked AppKit UTF-16
text conversion, but the native surface cannot request or return those values.
The bridge must not expose AppKit handles, clone whole documents, create another
state owner, wait for a worker, or turn a pull query into idle rendering.

## Decision

`alpine-platform-macos` owns the handle-free protocol on every target. One
`SurfaceEvent::Accessibility` carries a validated request through the existing
synchronous main-thread callback. `AppContext` admits at most one response for
that dispatch, and `SurfaceResponse` retains the response as an independent
bounded channel. Every response repeats request ID, operation kind, requested
revision, and observed revision.

Studio remains the sole semantic and text authority. Snapshot responses contain
nodes and metadata but no document text. Text is requested separately against
an exact document and buffer revision. Studio converts the UTF-16 endpoints,
checks the resulting byte count, and only then materializes at most 64 KiB.
Selection actions use the existing revision-checked editor transaction boundary.
Queries do not invalidate or submit a frame; an accepted selection change uses
the existing dirty-only frame path.

The private AppKit adapter and physical VoiceOver qualification remain Tasks
#252 and #253. This slice adds no native object, unsafe code, package, feature,
worker, queue, timer, polling loop, network path, plugin boundary, AI path,
telemetry, or startup work.

## Locked limits

| Resource | Limit | Failure behavior |
| --- | ---: | --- |
| Semantic nodes | 271 | Reject the snapshot |
| One node name | 4 KiB UTF-8 | Reject before publication |
| Aggregate referenced names | 256 KiB | Reject the snapshot |
| One text response | 64 KiB UTF-8 | Reject before materialization |
| Accessibility response per dispatch | 1 | Reject duplicate admission |
| Native or Studio handles in protocol | 0 | Architecture violation |
| Background waits or queues | 0 | Architecture violation |

## Atomic claims and evidence contract

- **AEP-0250-C01:** Every successful response matches one exact request ID,
  operation, and revision; stale actions and mismatched payloads fail before
  Studio mutation.
- **AEP-0250-C02:** Semantic and text ownership remains within locked node, name,
  and text ceilings, and snapshots never retain document text.
- **AEP-0250-C03:** Runtime response admission is single-assignment and exists
  only during synchronous foreground event dispatch.
- **AEP-0250-C04:** Studio answers current snapshot, text, selection, and action
  requests through its existing semantic and buffer owners; pull queries remain
  dirty-neutral and accepted selection changes use dirty-only rendering.

Constructor controls, tree validation, response mismatch and stale-revision
controls, UTF-16 Unicode tests, duplicate runtime admission, Studio production
dispatch, checked-range Kani evidence, finite TLA+ current-response and
single-assignment invariants with faulty controls, changed-line coverage, viable
mutation rejection, and hosted `ci-pass` are required.

## Explicit exclusions

AppKit element subclasses, native notifications, line and grapheme selectors,
text geometry, VoiceOver automation, Accessibility Inspector capture, physical
latency, multi-window semantics, AccessKit, plugins, AI, collaboration, cloud,
remote development, telemetry, and comparative performance claims are excluded.
Those native selectors and journeys remain dependency-ordered in Tasks #252 and
#253 without widening the ownership or byte ceilings accepted here.

## Reversal conditions

Supersede this AEP only if physical native testing proves that the pull protocol
cannot express required AppKit selectors, or an approved second platform makes
a shared adapter measurably smaller and safer than direct native translation.
