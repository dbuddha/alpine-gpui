# macOS accessibility decision ledger

## Include

- Ordered native marked-text discard and one Studio cancellation.
- Monotonic input epochs and stale/future callback rejection.
- Minimal revision-tagged activation for existing Studio commands.
- Focused-element forwarding, stable identifiers, and bounded element frames.
- Correct announcement, layout, and destruction notification semantics.
- A real Studio native process journey.
- A separate physical AX observer/action harness and human attestation.

## Exclude

- AccessKit as a shipping dependency at the inspected evidence level.
- A plugin-style action registry or second retained semantic tree.
- Arbitrary text-range geometry and multi-window behavior.
- AI, cloud, collaboration, telemetry, remote development, and plugins.
- VoiceOver automation or physical usability claims on hosted runners.

## Ordered implementation

Task #269 establishes IME focus epochs. Task #270 adds bounded actions, focus,
identity, and rectangles. Task #271 adds notification and destruction
semantics. Task #272 proves the real process journey. Task #273 owns physical
AX and VoiceOver evidence. Task #253 composes the final Requirement #37 result.

Reopen Decision #268 if physical evidence disproves activation/focus behavior,
AppKit requires another lifetime model, element rectangles are insufficient, or
the contract creates unbounded residency or idle work.
