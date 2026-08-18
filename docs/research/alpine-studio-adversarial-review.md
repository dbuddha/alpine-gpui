# Alpine Studio adversarial review

This review records the accepted 2026-08 implementation verdict for Alpine
GPUI and Alpine Studio. It evaluates correctness first, then performance,
resource efficiency, and delivery speed. Research history and future updates
belong to [Research #118](https://github.com/dbuddha/alpine-gpui/issues/118).
The current-state correction is tracked by
[Task #202](https://github.com/dbuddha/alpine-gpui/issues/202).

## Executive verdict

Alpine's strongest asset is its unusually explicit native renderer contract:
validated values, immutable scenes, structured failures, demand-driven pacing,
direct Metal presentation, exact resource accounting, semantic and CPU oracles,
and lifecycle qualification. Those foundations should remain narrow.

Since the original review, Alpine has crossed the renderer-only boundary. The
accepted main line now includes bounded asynchronous presentation, runtime and
input events, a local text model, CoreText shaping and glyph rendering, one-file
editing, clipboard and IME behavior, atomic save, a folder workspace, lazy file
tree, tabs, splits, search, command discovery, and crash-safe restoration. The
central weakness is now daily-driver completion and qualification: compiled
syntax, Rust intelligence, settings and keymap completion, native accessibility,
sustained dogfood, packaging, and defensible fixed-hardware evidence remain open.

## Keep, change, and defer

| Area | Keep | Change now | Defer or exclude | Primary effect |
| --- | --- | --- | --- | --- |
| Core contracts | Validated values, immutable revisions, structured errors, safe public boundaries | Add abstractions only when consumed by a Studio slice | General component or reactive framework | Correctness, delivery |
| Native lifecycle | One AppKit surface, CAMetalDisplayLink, visibility gating, latest-wins invalidation, deterministic close | Exercise only production paths and preserve bounded watchdog evidence | Multi-window runtime | Correctness |
| Presentation | Direct drawable presentation, three-drawable ceiling, zero idle work | Remove callback GPU waits using three completion-owned slots | Background rendering and deep queues | Performance, responsiveness |
| Scene | Deterministic semantic and CPU oracles, clips, glyph instances, ordered operations, and Direct Metal specialization | Stabilize realistic code-viewport traces and qualification workloads | Rich images, shadows, arbitrary paths, animation | Correctness, memory |
| Product state | Direct StudioApp to Workspace to Editor to Buffer ownership and explicit revisions | Finish language, settings, accessibility, and dogfood slices without a general component graph | GPUI entity compatibility and distributed state | Correctness, delivery |
| Text | Local snapshots, Unicode mappings, bounded undo, visible-range CoreText shaping, and byte-accounted caches | Qualify large files, IME, external changes, and sustained editing | Collaboration clocks and a custom rope | Correctness, memory |
| Product scope | Local-only editor-first boundary | Enforce excluded subsystems through binary and process audits | AI, collaboration, cloud, telemetry, plugins, remote, debugger, terminal, tasks, Git UI | Efficiency, delivery |
| Qualification | Exact trace identity and semantic admission | Separate adaptation, renderer stages, product journeys, memory, and exclusions | Headline averages and universal fastest-framework claims | Claim validity |

## Zed findings applied narrowly

The retained [Zed application case study](../case-studies/zed-editor.md) and
[GPUI renderer case study](../case-studies/zed-gpui.md) support these choices:

- demand-driven invalidation rather than continuous redraw;
- ephemeral element construction over retained application state;
- explicit layout, prepaint, and paint phases only where Studio consumes them;
- visible-range layout and bounded overscan for editor and uniform lists;
- current-frame and previous-frame text-layout reuse;
- primitive-specific batches and reusable instance buffers;
- removable, byte-accounted atlas entries;
- background results tagged with the revisions they were computed from.

Alpine does not adopt Zed's collaboration clocks, remote operation history,
deleted-text replication structures, live sharing, AI, cloud accounts,
extension host, marketplace, remote development, telemetry, or broad product
services. Zed source is research evidence, not copied implementation.

## Sublime findings applied narrowly

The [Sublime case study](../case-studies/sublime-editor.md) supports a focused
local-speed philosophy: custom rendering, batched GPU work, asynchronous save,
low-priority indexing, lazy non-critical work, graceful degradation, and
avoiding unnecessary redraw. Sublime's private text structure, cache design,
allocator, scheduler, and threading topology remain unknown and cannot justify
an Alpine implementation choice.

## Approved requirement anchors

| Requirement | Research-backed implementation boundary |
| --- | --- |
| [#32](https://github.com/dbuddha/alpine-gpui/issues/32) | One local workspace, virtualized tree, tabs, splits, navigation, bounded search, and crash-safe restoration |
| [#33](https://github.com/dbuddha/alpine-gpui/issues/33) | Local revisioned text, Unicode mappings, visible-range shaping, bounded glyph and line caches, and large-file behavior |
| [#34](https://github.com/dbuddha/alpine-gpui/issues/34) | Compiled syntax cohort, bounded search and symbols, and local revision-tagged rust-analyzer transport |
| [#35](https://github.com/dbuddha/alpine-gpui/issues/35) | Terminal, tasks, and Git UI remain outside daily-driver qualification |
| [#36](https://github.com/dbuddha/alpine-gpui/issues/36) | Central typed settings, themes, keymaps, and commands without runtime extension registration |
| [#37](https://github.com/dbuddha/alpine-gpui/issues/37) | Single-window native input, clipboard, IME, focus, accessibility, lifecycle recovery, and latency evidence |

## Execution order and hard gates

1. Preserve the completed production AppKit and deterministic close boundary.
2. Preserve the completed asynchronous, three-slot presentation contract.
3. Preserve and dogfood the implemented real-file editor and native input path.
4. Preserve and dogfood the implemented local workspace shell and restoration.
5. Reconcile the open compiled-language, LSP, settings, no-bloat, and
   accessibility PR stack onto current `main` in dependency order.
6. Complete local Rust intelligence, typed settings, restoration, and accessibility.
7. Dogfood the Alpine repositories with no known data-loss, lifecycle,
   unbounded-memory, idle-submission, IME, or accessibility defects.
8. Qualify only named renderer and product claims through the
   [comparator protocol](../quality/comparator-protocol.md).

The exact gate status and open PR order are retained in the
[daily-driver path](../use-cases/alpine-studio-highfidelity.md). M5 in the
current GitHub milestone scheme is not, by itself, the Studio daily-driver exit.

Correctness failure blocks timing. Performance regression blocks efficiency
claims. Memory optimization cannot omit behavior. Delivery shortcuts cannot
widen public API, dependency, unsafe, or product scope without a new accepted
decision.

## Fair performance and memory assessment

Renderer-only comparison starts after both systems hold semantically identical
prepared scenes. Adaptation is measured separately. Product comparison admits
only journeys with equal final bytes, selections, viewport, visible output,
input and IME outcome, accessibility, lifecycle work, and bounded residency.

Every accepted result binds `workload_identity_hash`, `environment_hash`, and
`exclusion_manifest_hash`; separates cold and warm samples; calibrates A/A
variance; randomizes AB and BA order; retains invalid runs; and reports p50,
p95, p99, effect size, and confidence intervals. Alpine-owned bytes and process
physical footprint are both required. A stable cache does not excuse total
footprint growth, and a low footprint does not excuse inaccurate accounting.

The permitted claims are Alpine GPUI versus pinned GPUI for named matched
renderer workloads, and Alpine Studio versus pinned Zed and externally measured
Sublime for named normalized local-editor journeys. Editor evidence never proves
that Alpine is the fastest general UI framework.

## Delivery risks that remain visible

- Text correctness, Unicode coordinate conversion, IME, and data-loss handling
  dominate the first editor risk.
- Removing the GPU wait changes ownership timing and must land before text while
  preserving close, completion reorder, device-loss, and memory-drain evidence.
- CoreText and AppKit accessibility add native unsafe boundaries that require
  narrow approved tasks and native tests.
- Rope, Unicode, grammar, serialization, ignore, JSON-RPC, and accessibility
  dependencies require separate measured approval before admission.
- Fixed-hardware superiority evidence does not exist until the accepted
  workload and environment windows are collected.
- The open Gate 5 PRs are stacked on pre-workspace branches and must be
  reconciled to current `main`; merging the stale stack directly risks dropping
  accepted workspace behavior.

These are sequencing constraints, not reasons to broaden the framework or add
speculative infrastructure.
