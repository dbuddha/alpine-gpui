# Current limitations and scope

Studio is a pre-release local editor for Apple Silicon macOS 15 or newer. It is
not qualified or distributed as a public daily driver. Physical typing latency,
VoiceOver, sustained dogfood and residency still require target-Mac acceptance.
Signing, notarization, updates and public release support remain future work.

The native text-input bridge reports the focused editor's UTF-16 selection,
bounded text, caret/first-line geometry and containing-glyph hits. Accessibility
range rectangles enclose supported visible line fragments, with a 256-fragment
limit; unavailable geometry returns no rectangle. Replacement callbacks validate
UTF-16 boundaries and reject stale document, focus and composition ownership.

This does not qualify physical candidate-window placement or VoiceOver editing.
Find/Replace fields expose their own native UTF-16 text, selection, projected
geometry and glyph queries. They support partial replacement, keyboard and mouse
selection, grapheme deletion, and horizontal caret scrolling within the existing
4 KiB field limit. Clicking outside dismisses Find and continues the click.
Their native Accessibility text selectors, clipboard and undo history are still
incomplete; VoiceOver editing is not qualified. Palette, quick-open, project-search
and symbol/rename fields still lack their own native text and geometry queries.
Editor composition replaces the displayed span, moves the
suffix and following lines, and shares projected coordinates with native text
geometry and glyph hits. Projection retains at most 1 MiB of boundary text plus
preedit combined; oversized projections revoke composition rather than exposing
text that differs from the scene. Physical IME, VoiceOver and presentation timing
remain acceptance work before claiming readiness.

The current implementation excludes collaboration, hosted AI, accounts, cloud sync, remote
development, telemetry, executable plugins, an extension marketplace, a debugger,
integrated terminal/task/Git UI, and multi-window qualification. Use external
terminal and Git tools. Linux and Windows test portable contracts; they do not
provide native Studio implementations.

The [capability probe](../alpine-capability-probe.md) records the approved macOS
terminal/editor/database/agent-workspace ambition. Its terminal, grid and dock
replays are non-shipping experiments, not implemented integrations. Network and
Unix-socket shipping restrictions remain until a scoped connectivity change.

Alpine does not ship GPUI, WGPU, a general async runtime, or a general reactive
entity/component framework. Keep bounded local ownership and Direct Metal.
Comparator feature parity does not authorize new product scope. Revisit an
exclusion through an explicit user decision describing the outcome, cost, ownership
and acceptance; no Capability/Requirement hierarchy or Project field is required.
