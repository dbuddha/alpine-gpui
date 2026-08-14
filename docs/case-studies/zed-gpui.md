# Zed GPUI and Zed

- Reviewed: 2026-08-09
- Research: [#18](https://github.com/dbuddha/alpine-gpui/issues/18)
- Revision: [`1271f8b0e8f3278eed5dd3fc12ad4bd30dce2c5d`](https://github.com/zed-industries/zed/tree/1271f8b0e8f3278eed5dd3fc12ad4bd30dce2c5d/crates/gpui)
- License: the `gpui` crate declares Apache-2.0
- Influence: conceptual, behavioral, and workload-based

## Findings

- **CS-ZED-001:** Entity-owned state with context-mediated mutation makes
  invalidation and ownership explicit enough to inspire Alpine's runtime model.
- **CS-ZED-002:** Request-layout, prepaint, and paint phases separate semantic
  construction from immutable renderer input and native submission.
- **CS-ZED-003:** Direct Metal specialization and headless rendering are
  compatible when the scene contract stays backend-neutral.
- **CS-ZED-004:** Zed's editor exercises dense text, virtualization, focus,
  multi-window, input, and diagnostic workloads suitable for Alpine dogfood.

Alpine rejects source compatibility, Zed workspace coupling, and inheriting
product-specific services. Production use is evidence of workload relevance,
not proof of Alpine correctness or performance.
