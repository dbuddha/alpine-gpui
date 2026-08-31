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

## Current-stable delta review at v1.17.2

Research [#95](https://github.com/dbuddha/alpine-gpui/issues/95) reviewed Zed
[`v1.17.2`](https://github.com/zed-industries/zed/tree/c8e44cfa7bda9b2e22c8d6934d78969352e7f61a)
without changing the immutable v1.15.0 comparator.

- The current [scene](https://github.com/zed-industries/zed/blob/c8e44cfa7bda9b2e22c8d6934d78969352e7f61a/crates/gpui/src/scene.rs)
  and [line-layout cache](https://github.com/zed-industries/zed/blob/c8e44cfa7bda9b2e22c8d6934d78969352e7f61a/crates/gpui/src/text_system/line_layout.rs)
  have the exact comparator blob identities `ea0f5d7e...` and `5e85cefb...`.
- The Metal sources moved from `gpui_macos` to `gpui_apple`. The current
  [renderer](https://github.com/zed-industries/zed/blob/c8e44cfa7bda9b2e22c8d6934d78969352e7f61a/crates/gpui_apple/src/metal_renderer.rs)
  differs by five `pub(crate)` to `pub` visibility changes, and the current
  [atlas](https://github.com/zed-industries/zed/blob/c8e44cfa7bda9b2e22c8d6934d78969352e7f61a/crates/gpui_apple/src/metal_atlas.rs)
  differs by one. No audited batching, completion, or atlas behavior changed.
- The current [profiler](https://github.com/zed-industries/zed/blob/c8e44cfa7bda9b2e22c8d6934d78969352e7f61a/crates/gpui/src/profiler.rs)
  has separate 16 MiB caps for the global frame-event deque and each thread's
  task-timing deque. The [debug overlay](https://github.com/zed-industries/zed/blob/c8e44cfa7bda9b2e22c8d6934d78969352e7f61a/crates/gpui/src/debug_overlay.rs)
  retains 1,000 draw-duration samples and paints directly into the scene to
  avoid self-invalidating.
- `record_present` runs immediately after `platform_window.draw` returns. Its
  timestamp is a useful framework endpoint, but it is not compositor-observed
  presentation or optical latency.
- Current frame demand explicitly re-arms the platform after dirty work,
  next-frame callbacks, throttling, and present-only demand. This corroborates
  Alpine's existing wake and coalescing contracts. It does not authorize a
  continuous loop or a present-only tail without physical latency and energy
  evidence.

Decision: adopt no source and add no shipping dependency. Carry the endpoint
distinction into the existing profiler and present-tail experiments only.

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

Alpine historically waited for GPU completion in the drawable callback. PRs
#135 and #136 replaced that path with a bounded three-slot asynchronous
completion ring. The source finding remains relevant as a regression guard;
physical latency and residency qualification remain open.

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

## Upstream radar review: 2026-08-31

This review answers a narrow decision question: did GPUI change after Alpine's
accepted comparator pin in a way that should alter Alpine's runtime, renderer,
diagnostics, or qualification plan?

### Identity and method

| Field | Value |
| --- | --- |
| Accepted comparator pin | `e17dc4f9d50db73a458b64dcce50ecd4878b98a3` (`v1.15.0`) |
| Previously reviewed stable point | `c8e44cfa7bda9b2e22c8d6934d78969352e7f61a` (`v1.17.2`) |
| Radar review head | `1662f5f3f6497c5f80830ccdca1edfd1fc0c6c6a` |
| Relevant source scope | `crates/gpui`, `crates/gpui_apple`, and `crates/gpui_macos` |
| Evidence level | E2: exact-tree and source-diff review, not an Alpine reproduction |
| Tracking issue | [Research #445](https://github.com/dbuddha/alpine-gpui/issues/445) |

The review compared exact Git trees and blobs, then inspected changed runtime,
text, profiler, benchmark, renderer, tests, and license sources. Relative to the
original comparator pin, the relevant scope contains 11 added, 42 changed, and
4 removed paths. Relative to the already reviewed `v1.17.2` stable point, only
2 paths were added and 32 changed. The latter delta is the decision-bearing
scope; the larger count is retained so the audit does not silently substitute a
newer baseline for the accepted comparator identity.

### Source-backed findings

| Finding | Observation | Alpine consequence |
| --- | --- | --- |
| Scene representation | `scene.rs` has the same blob at the prior stable point and radar head. | No new scene-layout or batching technique is available to adopt. Alpine retains its immutable structure-of-arrays scene. |
| Metal atlas | `metal_atlas.rs` has the same blob at the prior stable point and radar head. | No new atlas allocation, eviction, or upload evidence changes Alpine's corrected lookup-first and dirty-row plan. |
| Metal renderer | Runtime renderer changes are limited to extending test-only configuration gates with `bench-support`; no production submission or resource-lifetime behavior changed. | Keep benchmark instrumentation isolated. Do not infer a renderer improvement or change the Direct Metal backend. |
| Demand re-arming | A dirty window or queued next-frame callback explicitly schedules and wakes a subsequent frame after throttling or a completed request. Tests cover callbacks that would otherwise be stranded. | This corroborates Alpine's latest-demand-wins wake contract. It does not justify continuous rendering or a present-only tail. Zero idle submissions remain the default. |
| Foreground attribution | A bounded journal records foreground work, frame-pending boundaries, drawing, and presentation. Independent collectors retain discontinuity markers when entries are unavailable. | Preserve Alpine's stage-separated evidence and explicit omissions. Consider a smaller bounded journal only if #304 or a reproduced incident cannot be explained by current reports. |
| Hang classification | GPUI separates foreground occupancy from dirty-to-present delay. A slow presentation with little foreground work is deliberately not classified as a foreground hang. | Alpine must not label compositor or GPU delay as CPU/editor work. Typing analysis #331 must retain separate event, mutation, scene, commit, completion, and presentation stages. |
| Benchmark support | Benchmark reports add foreground count, total, maximum, percentiles, and frame-budget overruns while excluding draw and present work already reported separately. | The measurement boundary supports Alpine's existing no-double-counting rule. It is research input, not comparable timing evidence. |
| Line splitting | `LineLayout::split_at` partitions shaped runs, clones prefix and suffix glyph vectors, and rebases suffix indices and positions. | Defer. It is a utility for a concrete caller, not evidence of lower allocation or faster shaping, and Alpine has no measured need for the same API. |
| Licensing | GPUI-family `LICENSE-APACHE` blobs are unchanged across the compared trees. | License boundaries are unchanged. This review studies behavior and copies no source. |

Primary source anchors:

- [Window invalidation and pending-frame recording](https://github.com/zed-industries/zed/blob/1662f5f3f6497c5f80830ccdca1edfd1fc0c6c6a/crates/gpui/src/window.rs#L182-L213)
- [Demand re-arming after throttling and frame completion](https://github.com/zed-industries/zed/blob/1662f5f3f6497c5f80830ccdca1edfd1fc0c6c6a/crates/gpui/src/window.rs#L1550-L1675)
- [Bounded foreground-journal limits](https://github.com/zed-industries/zed/blob/1662f5f3f6497c5f80830ccdca1edfd1fc0c6c6a/crates/gpui/src/profiler/journal.rs#L32-L56)
- [Journal collector and discontinuity contract](https://github.com/zed-industries/zed/blob/1662f5f3f6497c5f80830ccdca1edfd1fc0c6c6a/crates/gpui/src/profiler/journal.rs#L885-L938)
- [Foreground hang attribution](https://github.com/zed-industries/zed/blob/1662f5f3f6497c5f80830ccdca1edfd1fc0c6c6a/crates/gpui/src/profiler/hang.rs#L1-L86)
- [Slow-presentation classification control](https://github.com/zed-industries/zed/blob/1662f5f3f6497c5f80830ccdca1edfd1fc0c6c6a/crates/gpui/src/profiler/hang.rs#L805-L836)
- [Line-layout split utility](https://github.com/zed-industries/zed/blob/1662f5f3f6497c5f80830ccdca1edfd1fc0c6c6a/crates/gpui/src/text_system/line_layout.rs#L128-L166)
- [Unchanged Apache license identity](https://github.com/zed-industries/zed/blob/1662f5f3f6497c5f80830ccdca1edfd1fc0c6c6a/crates/gpui/LICENSE-APACHE)

### Decision and limits

The decision is to retain Alpine's current architecture. Adopt the principles
of explicit demand re-arming, bounded diagnostic retention, discontinuity
reporting, and stage attribution. Do not import GPUI's journal, profiler,
runtime, line-layout utility, or source. Do not widen Alpine's product runtime
or add always-on diagnostic memory from this review.

The comparator remains pinned to `v1.15.0`; changing that identity would
invalidate retained fixtures and requires a separate accepted requalification.
Only the upstream radar baseline advances to the reviewed head so future scans
report new deltas instead of reopening this one. No code was executed and no
timing or memory samples were collected, so this review supports architecture
decisions at E2 and supports no performance claim. Revisit the decision when a
new radar issue identifies runtime or renderer changes, or when Alpine's own
typing evidence demonstrates a diagnostic gap that current bounded reports
cannot localize.
