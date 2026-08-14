# Zed GPUI and macOS renderer

- Reviewed: 2026-08-14
- Research: [#27](https://github.com/dbuddha/alpine-gpui/issues/27)
- Release: `v1.15.0`
- Revision: [`e17dc4f9d50db73a458b64dcce50ecd4878b98a3`](https://github.com/zed-industries/zed/tree/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui)
- License: the `gpui` crate declares Apache-2.0
- Influence: conceptual, behavioral, workload-based, and differential

## Reviewed surfaces

- `crates/gpui`: application and window contexts, entity ownership, elements,
  layout phases, scene construction, input dispatch, text, and headless tests.
- `crates/gpui/src/platform/mac`: AppKit event and window integration, display
  link behavior, Metal device and layer ownership, command submission, resource
  pools, completion, and offscreen readback.
- `crates/benchmarks`: Criterion-backed application contexts, dirty-to-draw and
  draw timing, invalidation counts, bounded task servicing, and headless Metal
  submission.
- GPUI visual-test and performance-test paths used by editor and UI crates.

At this revision, GPUI scenes cover shadows, quads, paths, underlines,
monochrome sprites, subpixel sprites, polychrome sprites, and embedded surfaces
in painter-ordered batches. The Metal path owns native buffers, textures,
pipelines, drawable acquisition, command buffers, and completion-driven reuse.
Headless rendering can submit to real Metal and read pixels back without making
window presentation equivalent to an offscreen result.

## Findings

- **CS-ZED-001:** Entity-owned state with context-mediated mutation makes
  invalidation and ownership explicit enough to inspire Alpine's runtime model.
- **CS-ZED-002:** Request-layout, prepaint, and paint phases separate semantic
  construction from immutable renderer input and native submission.
- **CS-ZED-003:** Direct Metal specialization and headless rendering are
  compatible when the scene contract stays backend-neutral.
- **CS-ZED-004:** Zed's editor exercises dense text, virtualization, focus,
  multi-window, input, and diagnostic workloads suitable for Alpine dogfood.
- **CS-ZED-005:** An exact renderer comparison needs the same pinned application
  workload, explicit GPUI-to-Alpine adaptation accounting, and a separate
  renderer-only timing boundary after both scenes are prepared.
- **CS-ZED-007:** Zed's benchmark work provides useful dirty-to-draw, draw-time,
  invalidation, and headless Metal patterns, but headless submission and proxy
  frame budgets do not establish GPU completion, presentation, or
  input-to-photon latency.

## Patterns adopted and rejected

Alpine adopts explicit entity mutation, separated layout and paint phases,
immutable renderer input, demand-driven invalidation, native headless
validation, completion-aware resource ownership, and workload-oriented
benchmark contexts as research prompts.

Alpine rejects drawing once per benchmark flush as a frame-pacing oracle,
dirty-to-draw measurements that omit queueing as complete input latency,
synthetic budget overruns as display-deadline proof, and headless submission
without GPU completion as total GPU time. Each Alpine metric names its start,
end, excluded queueing or presentation stages, and native evidence.

Alpine rejects source compatibility, Zed workspace coupling, automatic upstream
synchronization, and inheriting product-specific services. Production use is
evidence of workload relevance, not proof of Alpine correctness or performance.
