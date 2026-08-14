# awesome-gpui workload survey

- Reviewed: 2026-08-09
- Research: [#24](https://github.com/dbuddha/alpine-gpui/issues/24)
- Revision: [`cf11f85a1420dfc5a7f64bc159aacba8133a2f35`](https://github.com/zed-industries/awesome-gpui/tree/cf11f85a1420dfc5a7f64bc159aacba8133a2f35)
- License: CC0-1.0
- Influence: workload discovery

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
