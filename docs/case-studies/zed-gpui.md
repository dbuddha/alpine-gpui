# Zed GPUI and macOS renderer

- Reviewed: 2026-08-15
- Research anchors: [#27](https://github.com/dbuddha/alpine-gpui/issues/27), [#113](https://github.com/dbuddha/alpine-gpui/issues/113), [#115](https://github.com/dbuddha/alpine-gpui/issues/115)
- Release: `v1.15.0`
- Revision: [`e17dc4f9d50db73a458b64dcce50ecd4878b98a3`](https://github.com/zed-industries/zed/tree/e17dc4f9d50db73a458b64dcce50ecd4878b98a3)
- License boundary: pinned `gpui` and `gpui_macos` manifests declare Apache-2.0
- Influence: conceptual, behavioral, workload-based, and differential
- Evidence strength: pinned primary source plus official engineering articles

## Research question

Which GPUI rendering and frame-lifecycle techniques should Alpine preserve,
which are comparator-specific, and which cannot support a performance claim
without additional completion, presentation, memory, and semantic evidence?

## Implemented GPUI pipeline at the pin

GPUI keeps retained application state but constructs an ephemeral element and
paint result when a window is dirty. Its window invalidator records dirty state,
dirty views, update count, and coalesced invalidations
([window.rs lines 117-176](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui/src/window.rs#L117-L176)).
The draw pipeline separates request-layout, prepaint, and paint. The result is a
scene with ordered operations and separate arrays for shadows, quads, paths,
underlines, glyph sprites, images, and surfaces
([scene.rs lines 41-52](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui/src/scene.rs#L41-L52)).
The scene sorts primitive-specific storage by draw order and texture identity
before producing painter-ordered batches
([scene.rs lines 151-188](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui/src/scene.rs#L151-L188)).

On macOS, GPUI writes primitive instances into pooled Metal buffers, attaches a
completion handler that returns the buffer to the pool, then commits and
presents
([metal_renderer.rs lines 472-529](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui_macos/src/metal_renderer.rs#L472-L529)).
The synchronous image paths wait for command completion because the CPU reads
pixels, while the benchmark submission path intentionally commits without
waiting
([metal_renderer.rs lines 557-605](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui_macos/src/metal_renderer.rs#L557-L605),
[lines 643-648](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui_macos/src/metal_renderer.rs#L643-L648)).

## Stable finding registry

These identifiers are immutable because accepted AEP claims reference them.
The detailed sections below add evidence without renaming the findings.

- **CS-ZED-001:** Application-owned state and context-mediated mutation make
  invalidation and ownership explicit enough to inform Alpine's smaller direct
  runtime model.
- **CS-ZED-002:** Request-layout, prepaint, and paint separate semantic
  construction from immutable renderer input and native submission.
- **CS-ZED-003:** Direct Metal specialization and headless rendering coexist
  when the scene contract remains backend-neutral.
- **CS-ZED-004:** Zed's editor provides dense text, virtualization, focus,
  input, diagnostic, and lifecycle workloads for Alpine dogfood.
- **CS-ZED-005:** Exact renderer comparison requires one pinned workload,
  explicit adaptation accounting, and a renderer-only boundary after both
  scenes are prepared.
- **CS-ZED-006:** Product timing is invalid until document state, visible
  semantics, accessibility, lifecycle, and resource accounting are equivalent.
- **CS-ZED-007:** Dirty-to-draw and headless submission are useful stages but do
  not establish GPU completion, presentation, or input-to-photon latency.
- **CS-ZED-008:** A daily-driver editor exposes framework weaknesses that a
  solid-quad sample cannot, including text, IME, accessibility, recovery, and
  long-lived retention.
- **CS-ZED-009:** GPL Zed application source and artifacts require isolation
  from proprietary Alpine code, even though pinned GPUI crates declare
  Apache-2.0.

## Findings and Alpine decisions

### ZED-GPUI-001: dirty-state coalescing is foundational

The dirty flag and invalidation accumulator prove that multiple state changes
can be represented by one requested draw. Alpine keeps latest-revision
invalidation and zero-idle pacing. A clean, hidden, occluded, zero-sized,
closing, or stopped surface submits no work.

### ZED-GPUI-002: layout, prepaint, and paint are useful phase boundaries

Alpine should add phases only when consumed by Studio. Layout computes geometry
and visible ranges, prepaint resolves clips and text-layout reuse, and paint
emits immutable primitive arrays. Alpine does not need a general component
framework or GPUI-compatible element trait to preserve these boundaries.

### ZED-GPUI-003: structure-of-arrays scenes enable narrow batching

The pinned scene separates primitive arrays while retaining painter order. For
the one-file editor, Alpine needs quads, clips, monochrome glyphs, and ordered
paint operations. Shadows, arbitrary paths, rich images, embedded surfaces,
and animation remain deferred until a qualified product slice requires them.

### ZED-GPUI-004: line-layout reuse should avoid text materialization

The pinned line-layout cache has current-frame and previous-frame maps
([line_layout.rs lines 392-415](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui/src/text_system/line_layout.rs#L392-L415)),
moves layouts between frame caches
([lines 444-500](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui/src/text_system/line_layout.rs#L444-L500)),
and offers a content-hash probe that does not materialize contiguous text
([lines 630-666](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui/src/text_system/line_layout.rs#L630-L666)).

Alpine adopts the two-frame reuse pattern behind an Alpine-owned CoreText
interface, with exact byte accounting, hit and miss counts, a 32 MiB hard
ceiling, and collision-safe identity. A hit must avoid text materialization and
shaping. Hash-only reuse is not accepted without a content identity guard.

### ZED-GPUI-005: completion-driven resource reuse is the correct ownership model

GPUI returns its instance buffer from a command-buffer completion handler.
Alpine adopts completion-held frame slots, but caps ownership at three and
publishes current and peak in-flight frames, upload capacity, allocated bytes,
retained bytes, and completion status. No unbounded command-buffer or upload
buffer queue is permitted.

### ZED-GPUI-006: atlas removal is necessary but not a complete memory policy

The pinned atlas removes keys, deallocates tiles, and recycles an unreferenced
texture slot
([metal_atlas.rs lines 62-91](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui_macos/src/metal_atlas.rs#L62-L91)).
Alpine starts with a monochrome A8 atlas, a 16 MiB default budget, exact byte
accounting, removable entries, explicit pressure handling, and post-close drain.
The budget is an Alpine design decision, not a claim about Zed.

### ZED-GPUI-007: a main-thread GPU completion wait is disqualifying for Studio

Zed reports that `waitUntilCompleted` blocked much longer in direct-display mode
and caused visible jank
([Zed, 2022](https://zed.dev/blog/120fps)). The pinned renderer uses
`wait_until_scheduled` only for transaction-coordinated presentation and does
not wait in its normal present path
([metal_renderer.rs lines 480-487](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui_macos/src/metal_renderer.rs#L480-L487)).

Alpine's current drawable callback still waits for GPU completion. Replacing it
with a bounded three-slot asynchronous completion ring is the next performance
foundation, but requires an approved AEP because it changes concurrency,
ownership, accounting, and error publication.

### ZED-GPUI-008: submission is not completion or presentation

GPUI's headless benchmark path explicitly commits without waiting. It is useful
for CPU scene-build and submission comparisons, but cannot establish GPU time,
display time, or input-to-photon latency. Alpine reports adaptation, scene
build, upload, encode, commit, GPU completion, presentation, and optical stages
separately.

## Apple contract check

Apple describes `CAMetalDisplayLink` as a run-loop-bound, variable-refresh
callback source that can be paused and invalidated. Its update provides a
drawable and deadline; commands must be encoded and committed before direct
`present()` ([Apple, current](https://developer.apple.com/documentation/quartzcore/cametaldisplaylink)).
Apple also states that GPU rendering may continue after the callback deadline
according to preferred frame latency
([targetTimestamp](https://developer.apple.com/documentation/quartzcore/cametaldisplaylink/update/targettimestamp)).
This directly contradicts a requirement to wait for completion inside the
callback.

## Adopt, adapt, and reject matrix

| Pattern | Decision | Required Alpine guard |
| --- | --- | --- |
| Dirty-state coalescing | Adopt | Latest revision, no idle callbacks |
| Layout, prepaint, paint | Adapt | Add only for Studio slices |
| Primitive-specific arrays | Adopt narrowly | Quads, clips, glyphs first |
| Current and previous text layouts | Adopt | Byte ceiling and collision-safe identity |
| Completion-driven buffer pool | Adopt | Three slots and exact retained-byte accounting |
| Removable atlas tiles | Adopt | Budget, eviction, pressure, drain |
| GPUI entity/component compatibility | Reject | Direct Studio ownership |
| Main-thread completion wait | Reject | Asynchronous terminal completion |
| Headless commit as total GPU time | Reject | Separate completion and presentation endpoints |
| GPUI product services | Reject | Local editor only |

## Sources

- [Pinned GPUI scene](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui/src/scene.rs): scene storage, ordering, and batches.
- [Pinned GPUI window](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui/src/window.rs): invalidation and draw phases.
- [Pinned line-layout cache](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui/src/text_system/line_layout.rs): frame-local reuse.
- [Pinned Metal renderer](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui_macos/src/metal_renderer.rs): submission, presentation, completion, and readback.
- [Pinned Metal atlas](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui_macos/src/metal_atlas.rs): tile and texture reclamation.
- [Optimizing GPUI for 120 FPS](https://zed.dev/blog/120fps): direct-display completion-wait failure.
- [Apple CAMetalDisplayLink](https://developer.apple.com/documentation/quartzcore/cametaldisplaylink): platform pacing contract.
