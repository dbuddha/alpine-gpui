# macOS accessibility findings

| ID | Strength | Finding | Alpine consequence |
| --- | --- | --- | --- |
| AX-253-001 | Observed | Alpine has bounded revisioned pull transport, stable node IDs, weak native element ownership, generation revocation, and hard semantic limits. | Preserve these contracts. |
| AX-253-002 | Observed | Focus loss cancelled Rust completion but not IME composition for editor, find, quick open, command palette, or project search. | Add Task #269 input epochs and ordered cancellation. |
| AX-253-003 | Observed | Selection mutation exists, but tabs, files, commands, diagnostics, save, and close are not operable through accessibility. | Add only the bounded activation vocabulary in #270. |
| AX-253-004 | Observed | Announcement posting lacks complete user-info semantics, and removed elements lack destruction evidence. | Implement post-borrow notification and destruction in #271. |
| AX-253-005 | Observed | Focus exists internally, but external focused-element forwarding and stable external identifiers are absent. | Add focused child, identity, and bounded rectangles in #270. |
| AX-253-006 | Observed | The native adapter fixture is not the real `StudioApp` journey. | Add a real process E2E in #272. |
| AX-253-007 | Observed | Hosted counters show post intentions, not delivery to assistive technology. | Keep physical AX and VoiceOver evidence in #273. |
| AX-253-008 | Inference | Bounded semantic rectangles are needed for practical external navigation and hit testing. | Include element rectangles, but exclude arbitrary text-range geometry. |
| AX-253-009 | Inference | Main-thread serialization does not identify an obsolete conversion session after refocus. | Carry one monotonic input epoch and reject stale callbacks. |
| AX-253-010 | Disconfirming | AccessKit is a useful mechanism reference, but the inspected revision has no macOS integration-test directory. | Do not add the dependency or cite it as qualification evidence. |

Physical evidence must still determine whether activation establishes keyboard
focus, whether element rectangles suffice for daily-driver navigation, and
whether notification delivery remains bounded under churn.
