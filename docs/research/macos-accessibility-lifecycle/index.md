# macOS accessibility and input lifecycle research

- Research record: [#267](https://github.com/dbuddha/alpine-gpui/issues/267)
- Accepted decision: [#268](https://github.com/dbuddha/alpine-gpui/issues/268)
- Evidence level: E2, source-triangulated architecture research
- Alpine source inspected: `53bf751deff87d26811bd1d66fa6fb0d375f53d2`
- Zed source pin: `e17dc4f9d50db73a458b64dcce50ecd4878b98a3`
- AccessKit source pin: `2dfdd7b92e68edd4276841a5061f31ffc77e718b`

## Decision question

What is the smallest correct macOS VoiceOver and accessibility lifecycle that
can qualify Alpine Studio without test theater, a second semantic tree,
unbounded CI cost, or product weight unrelated to a local daily driver?

This package retains decision-grade findings and protocols. It is not a
VoiceOver run, external AX observation, memory soak, latency result, or
qualification report.

## Package map

- [Pinned source map](source-map.md)
- [Detailed findings](findings.md)
- [Qualification experiments](experiments.md)
- [Decision ledger](decisions.md)

## Answer

Keep one Alpine-owned semantic model and the existing bounded revisioned pull
transport. Add one monotonic input epoch, a minimal activation action, focused
element forwarding, stable identifiers, bounded element rectangles, correct
notification and destruction semantics, a real Studio process journey, and a
separate trusted physical AX and VoiceOver lane. Do not add AccessKit, another
semantic tree, arbitrary text geometry, plugins, AI, cloud, telemetry, or
hosted VoiceOver automation.
