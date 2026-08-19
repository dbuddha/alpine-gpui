# Alpine Studio adversarial review

This review records the accepted 2026-08 implementation verdict for Alpine
GPUI and Alpine Studio. It evaluates correctness first, then performance,
resource efficiency, and delivery speed. Research history and future updates
belong to [Research #118](https://github.com/dbuddha/alpine-gpui/issues/118).
The current-state correction is tracked by
[Task #225](https://github.com/dbuddha/alpine-gpui/issues/225).

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
central weakness is now daily-driver completion and qualification. Compiled
syntax, typed settings, shortcuts, the no-bloat boundary, revisioned
accessibility semantics, bounded local LSP transport, pinned rust-analyzer
qualification, and visible Rust diagnostics are merged. Completion, richer
Rust intelligence, configuration reload and migration, native VoiceOver and
lifecycle qualification, sustained dogfood, packaging, and defensible
fixed-hardware evidence remain open.

## Keep, change, and defer

| Area | Keep | Change now | Defer or exclude | Primary effect |
| --- | --- | --- | --- | --- |
| Core contracts | Validated values, immutable revisions, structured errors, safe public boundaries | Add abstractions only when consumed by a Studio slice | General component or reactive framework | Correctness, delivery |
| Native lifecycle | One AppKit surface, CAMetalDisplayLink, visibility gating, latest-wins invalidation, deterministic close | Exercise only production paths and preserve bounded watchdog evidence | Multi-window runtime | Correctness |
| Presentation | Direct drawable presentation, three completion-owned slots, zero idle work | Finish native close, compositor, idle, and lifecycle evidence without reintroducing callback waits | Background rendering and deep queues | Performance, responsiveness |
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
5. Complete bounded Rust completion, hover and navigation, rename and
   formatting, and symbols through Tasks #218 through #221.
6. Complete configuration reload and migration through Task #222, then close
   the native compositor, lifecycle soak, production journey, and idle-energy
   leaves #234 through #237 under Task #72 before Task #223 completes native
   VoiceOver and lifecycle qualification.
7. Execute dogfood capture, sustained sessions, interaction baselines,
   residency, and final acceptance through leaves #238 through #242 under Task
   #224, with no known data-loss, lifecycle, unbounded-memory, idle-submission,
   IME, or accessibility defects.
8. Qualify only named renderer and product claims through the
   [comparator protocol](../quality/comparator-protocol.md).

The exact gate status and open PR order are retained in the
[daily-driver path](../use-cases/alpine-studio-highfidelity.md). M5 is the
selected Apple Silicon macOS daily-driver behavior and dogfood gate. M7 is the
separate supported, packaged, fixed-hardware-qualified version 1 gate.

Milestone and Project item totals are not readiness percentages because they
contain capabilities, requirements, research, defects, and leaf tasks. Only
closed evidence-producing leaves count toward a scope-pinned burn-up; parent
progress is derived from those leaves and their acceptance contracts.

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

- Completion currently depends on a pinned real rust-analyzer response and
  must distinguish server readiness, supported item admission, and stale
  revision rejection without retries or polling that create idle work.
- Native VoiceOver and lifecycle qualification remains blocked by the residual
  single-window surface evidence decomposed as Tasks #234 through #237 under
  Task #72.
- Studio changed-line diagnostics do not yet cover `apps/` with the same
  actionable contract as `crates/`; Defect #232 must close before dogfood can
  rely on that gate.
- Nightly Miri assurance remains open in Defect #183 until its bounded shards
  pass authoritatively without unsupported process E2E or unbounded duration.
- Fixed-hardware superiority evidence does not exist until the accepted
  workload and environment windows are collected.
- Dogfood Tasks #238 through #242 have not yet established startup, input,
  scrolling, language, cache-churn, residency, post-close, and final M5
  acceptance evidence.

These are sequencing constraints, not reasons to broaden the framework or add
speculative infrastructure.
