# Current limitations and scope

Studio is a pre-release local editor for Apple Silicon macOS 15 or newer. It is
not qualified or distributed as a public daily driver. Physical typing latency,
VoiceOver, sustained dogfood and residency still require target-Mac acceptance.
Signing, notarization, updates and public release support remain future work.

The current scope excludes collaboration, hosted AI, accounts, cloud sync, remote
development, telemetry, executable plugins, an extension marketplace, a debugger,
integrated terminal/task/Git UI, and multi-window qualification. Use external
terminal and Git tools. Linux and Windows test portable contracts; they do not
provide native Studio implementations.

Alpine does not ship GPUI, WGPU, a general async runtime, or a general reactive
entity/component framework. Keep bounded local ownership and Direct Metal.
Comparator feature parity does not authorize new product scope. Revisit an
exclusion through an explicit user decision describing the outcome, cost, ownership
and acceptance; no Capability/Requirement hierarchy or Project field is required.
