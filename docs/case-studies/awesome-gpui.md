# awesome-gpui workload survey

- Reviewed: 2026-08-27
- Research: [#24](https://github.com/dbuddha/alpine-gpui/issues/24)
- Revision: [`cf11f85a1420dfc5a7f64bc159aacba8133a2f35`](https://github.com/zed-industries/awesome-gpui/tree/cf11f85a1420dfc5a7f64bc159aacba8133a2f35)
- License: CC0-1.0
- Influence: workload discovery
- Current review: [`f3889e71920ffbe8affa0f133c3db6ce6b06af76`](https://github.com/zed-industries/awesome-gpui/tree/f3889e71920ffbe8affa0f133c3db6ce6b06af76), Research [#100](https://github.com/dbuddha/alpine-gpui/issues/100)

## Findings

- **CS-AWESOME-001:** Editors and terminals stress text shaping, selection,
  input latency, IME, accessibility, and virtualized scrolling.
- **CS-AWESOME-002:** Database clients and large tables stress bidirectional
  virtualization, retained memory, focus, resize, and dense interaction.
- **CS-AWESOME-003:** Media and whiteboard applications stress embedded
  surfaces, transforms, clipping, frame pacing, and device recovery.
- **CS-AWESOME-004:** Multi-window tools stress state ownership, wake ordering,
  teardown, display scale, and cross-window resource policy.

Catalog presence and popularity are not maturity signals. Each selected dogfood
application needs its own Capability, acceptance workloads, and platform gates.

## Current catalog delta

The bounded `657169337a19a5b27f9aa7e53811e6f82b7f213c` to
`f3889e71920ffbe8affa0f133c3db6ce6b06af76` review contains 21 commits and
changes only `README.md` and `projects.json`. It adds catalog entries and
updates descriptions, status, and popularity metadata. It changes no
framework source, renderer source, test, benchmark protocol, or license file.

Decision: retain awesome-gpui as discovery-only evidence. Create no Alpine
implementation, dependency, or performance claim from this delta.
