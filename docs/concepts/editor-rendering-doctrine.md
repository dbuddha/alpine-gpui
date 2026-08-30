# Alpine editor rendering doctrine

This document owns Alpine's stable editor-rendering direction. GitHub issues
and Project fields own live delivery status. AEPs own substantial public API,
native ownership, dependency, unsafe, and qualification-policy changes.

## Product and rendering target

Alpine Studio is a full graphical native editor with terminal-like
implementation discipline. It is not a terminal emulator, fixed-cell TUI,
browser runtime, game engine, or GPUI compatibility layer.

The v1 composition is:

- An AppKit application shell with native input, focus, IME, clipboard,
  accessibility, lifecycle, and window semantics.
- A specialized row-oriented text canvas that builds visible lines plus bounded
  overscan.
- Purpose-built rows, columns, splits, overlays, and uniform virtual lists for
  editor chrome.
- Immutable scenes containing ordered quads, clips, glyphs, and a small
  Alpine-owned icon vocabulary.
- A demand-driven Direct Metal renderer with zero idle submissions and bounded
  asynchronous frame ownership.

Monospaced code layout should use grid-like arithmetic where it is correct, but
the product retains smooth pixel scrolling, Unicode shaping, grapheme
navigation, font fallback, bidirectional text, pointer hit testing, overlays,
and native accessibility semantics. ASCII-only or fixed-cell shortcuts cannot
substitute for these contracts.

## CPU and GPU responsibilities

The GPU does not make an editor fast by itself. The CPU owns event admission,
editor mutation, visible-range selection, shaping, layout, clipping, scene
construction, upload planning, and Metal encoding. The GPU transforms and
clips primitives, samples glyph and icon atlases, blends, and writes pixels.
AppKit and Core Animation own the final presentation boundary.

```text
AppKit event
    -> revision-checked Studio mutation
    -> visible-range layout and shaping reuse
    -> immutable Alpine scene
    -> bounded upload and Metal encoding
    -> GPU terminal result
    -> actual drawable presentation
```

At 120 Hz, relevant work across this complete path must fit the calibrated
8.33 ms display interval. Alpine measures each stage separately and never uses
GPU completion alone as an input-latency claim.

## Locked v1 boundaries

- Direct Metal is the Apple Silicon macOS shipping renderer.
- AppKit, CoreText, Core Animation, accessibility, and Metal stay behind narrow
  Alpine-owned interfaces.
- Rendering remains demand driven. A correct idle editor submits zero frames.
- Scenes are immutable, painter ordered, free of application objects, and free
  of native handles.
- Primitive, frame, worker, search, language, and cache ownership is bounded.
- Text work is visible range plus bounded overscan, with current-frame and
  previous-frame layout reuse.
- Glyph lookup precedes rasterization. Warm unchanged viewports perform no
  glyph rasterization, atlas publication, or atlas upload.
- Accessibility semantics are separate from visual primitives and remain a
  first-class correctness gate.
- Alpine Studio remains local only, with no collaboration, hosted AI, cloud,
  telemetry, plugin host, marketplace, or remote-development subsystem.

Any change to these boundaries requires an accepted issue and the applicable
AEP or dependency process. Visual novelty or comparator architecture alone is
not sufficient justification.

## Adaptation and comparator policy

Alpine adapts GPUI principles, not GPUI's complete implementation. Accepted
principles include retained application state, ephemeral visual construction,
explicit layout and paint stages, visible-range work, shaping reuse, batched
primitives, bounded resources, and demand-driven invalidation. Alpine does not
adopt GPUI's entity graph, global registries, reactive compatibility surface,
collaboration state, or product services without an accepted Studio need.

WGPU remains an isolated research and differential-validation path. Equivalent
scenes may use it to test clipping, glyph sampling, resource lifetime, resize,
and recovery semantics. WGPU, `wgpu-hal`, Naga, WGSL, and WebGPU APIs are not
shipping dependencies in v1, and adaptation cost is reported separately.

Only editor-relevant game-rendering techniques are admissible: bounded triple
buffering, explicit resource lifetime, deadline measurement, reusable upload
memory, primitive batching, offline preparation of built-in assets, and GPU
profiling. Alpine rejects continuous game loops, render graphs, ECS render
worlds, 3D asset pipelines, material systems, scene streaming, physics,
MetalFX, and generalized animation infrastructure.

The [lineage package](../research/alpine-lineage/index.md) records whether each
mechanism is adapted, independently convergent, Alpine-original,
comparator-only, rejected, or deferred. Source influence does not count as
proof of correctness or superiority.

## Performance and memory doctrine

Alpine optimizes in this order:

1. Correct editor, Unicode, IME, accessibility, lifecycle, and persistence
   behavior.
2. End-to-end responsiveness and display-deadline reliability.
3. CPU, GPU, and memory efficiency with bounded ownership and post-close drain.
4. Delivery speed within the earlier gates.

Optimization follows measured attribution. The required stage vocabulary
separates event admission, mutation, selection transformation, snapshot,
visible-range layout, shaping, atlas lookup, miss rasterization, scene build,
atlas publication, upload, encode, commit, GPU completion, display-link
deadline, and actual presentation. Required resource evidence includes
allocation count, process footprint, private dirty memory, CPU and GPU atlas
bytes, staging capacity, queue depth, cache bytes, peak residency, steady-state
slope, and post-close delta.

No architecture is justified by looking lightweight. A reusable element layer,
texture atlas change, builder reuse, batching split, or scheduling change is
accepted only when repeated Studio use establishes the contract or evidence
identifies a correctness, latency, or residency bottleneck.

## Evidence and delivery chain

| Gate | Required result |
| --- | --- |
| Exact-main assurance | Aggregate CI and terminal Nightly are green for the exact revision under qualification. |
| Typing latency | [#304](https://github.com/dbuddha/alpine-gpui/issues/304) and [#331](https://github.com/dbuddha/alpine-gpui/issues/331) retain physical release event-to-present distributions and measured attribution. |
| Native semantics | [#253](https://github.com/dbuddha/alpine-gpui/issues/253) and [#273](https://github.com/dbuddha/alpine-gpui/issues/273) retain physical VoiceOver, Accessibility Inspector, keyboard, IME, lifecycle, latency, and residency evidence. |
| Private dogfood | [#238](https://github.com/dbuddha/alpine-gpui/issues/238) through [#242](https://github.com/dbuddha/alpine-gpui/issues/242) retain revision-pinned journeys, baselines, residency, incidents, and fail-closed M5 acceptance. |
| Renderer qualification | [#353](https://github.com/dbuddha/alpine-gpui/issues/353) and [#53](https://github.com/dbuddha/alpine-gpui/issues/53) establish realistic semantic equivalence before calibrated E4 timing and memory claims. |

Hosted timing is diagnostic. Physical thresholds require A/A calibration,
revision and environment identity, raw samples, invalidation rules, and accepted
statistical analysis. Claims name the exact hardware, workload, behavior,
metric, and exclusions. Alpine does not publish a universal fastest-framework,
120 FPS, or memory-superiority claim from incomplete or editor-only evidence.

## Explicit exclusions

This doctrine does not authorize a new public API, dependency, renderer,
platform, framework abstraction, plugin surface, game process, or performance
claim. It does not close an issue or milestone. Missing, stale, invalid, or
inconclusive evidence remains red in GitHub Project and acceptance reports.
