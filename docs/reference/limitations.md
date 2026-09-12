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
Find/palette and other overlay fields do not yet expose their own native text and
geometry queries. Point lookup during marked text remains unavailable; composition
painting still overlays preedit on the original line. Physical IME, VoiceOver and
presentation timing therefore remain acceptance work before claiming readiness.

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
