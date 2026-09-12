# Current limitations and scope

Studio is a pre-release local editor for Apple Silicon macOS 15 or newer. It is
not qualified or distributed as a public daily driver. Physical typing latency,
VoiceOver, sustained dogfood and residency still require target-Mac acceptance.
Signing, notarization, updates and public release support remain future work.

Native text input is incomplete: `NSTextInputClient.selectedRange` returns a
zero-length range at offset zero, the substring callback returns no text, the first-rectangle callback returns a
fixed 1x1 rectangle, and point-to-character lookup returns zero. These do not yet
describe the editor's actual selection or caret geometry. Accessibility text-range
geometry is also unsupported. Synthetic IME event tests and a bounded AX tree
therefore do not qualify candidate-window placement or VoiceOver editing. Complete
the native selection/text/geometry bridge and verify it on the physical Mac before
claiming these journeys are ready.

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
