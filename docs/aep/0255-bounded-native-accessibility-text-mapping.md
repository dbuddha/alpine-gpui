# AEP 0255: Bounded native accessibility text mapping

- Status: Accepted
- Decision: [#255](https://github.com/dbuddha/alpine-gpui/issues/255)
- Task: [#252](https://github.com/dbuddha/alpine-gpui/issues/252)
- Requirement: [#37](https://github.com/dbuddha/alpine-gpui/issues/37)
- Parent tasks: [#130](https://github.com/dbuddha/alpine-gpui/issues/130), [#223](https://github.com/dbuddha/alpine-gpui/issues/223)

## Context

AppKit text accessibility asks for the logical line containing a UTF-16 index,
the UTF-16 range of a logical line, and the extended grapheme range containing
an index. AEP-0250 intentionally exposes neither full document text nor native
handles. Reconstructing these answers in the AppKit adapter would therefore
require duplicate text ownership, fabricated values, or an unbounded text pull.

The existing grapheme-index helpers materialize a contiguous document string.
That cost is unsuitable for a native selector whose memory use must remain
independent of document size.

## Decision

The handle-free accessibility protocol adds three exact, revision-bound
operations: `LineForIndex`, `RangeForLine`, and `RangeForIndex`. Responses repeat
request identity, operation kind, requested revision, and observed revision.
Their payload is one `usize` line or one existing `AccessibilityTextRange`.

Studio remains the only text-coordinate authority. It answers line requests
from immutable line metadata and answers grapheme requests with
`GraphemeCursor` over Ropey chunks. Traversal may inspect the context required
by Unicode segmentation but does not assemble a contiguous document or retain
text in the response. A final document index maps to the final logical line and
an empty grapheme range at the end.

The native adapter checks every `NSInteger` and `NSRange` conversion. It does
not advertise `rangeForPosition`, `frameForRange`, or another geometry selector
until a separate accepted scene-geometry contract can answer them truthfully.

## Atomic claims and evidence contract

- **AEP-0255-C01:** Every successful mapping response matches one exact request,
  operation, and revision; kind mismatches and stale revisions fail closed.
- **AEP-0255-C02:** Line and grapheme mappings are exact for Unicode, CRLF,
  empty lines, final boundaries, surrogate pairs, and combining sequences.
- **AEP-0255-C03:** Mapping ownership remains in `BufferSnapshot`, returns only
  scalar metadata, and allocates no text proportional to document size.

Constructor and payload controls, checked-range Kani evidence, independent
String differential tests, chunk-boundary Unicode tests, changed-line coverage,
viable mutation rejection, and hosted `ci-pass` are required before merge.
Native selector admission and lifecycle evidence remain part of Task #252.
The generic AEP-0250 `ResponseMatchesRequest` model remains applicable because
it abstracts an operation behind one active request identity and revision;
operation-kind discrimination is exercised by the concrete Rust unit control.

## Explicit exclusions

Full-document `accessibilityValue`, arbitrary text geometry, native element
publication, VoiceOver automation, AccessKit, multi-window semantics, plugins,
AI, collaboration, cloud, remote development, telemetry, and comparative
performance claims are excluded from this protocol slice.

## Reversal conditions

Supersede this AEP only if native testing proves these mappings cannot express
the required selectors without widening retained data, or if an accepted
bounded scene-geometry contract can support currently excluded selectors.
