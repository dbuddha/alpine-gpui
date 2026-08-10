# GPUI Ecosystem Analysis

- Research date: 2026-08-09
- Purpose: identify concepts, failure modes, and conformance material for Rock
  GPUI without selecting an upstream implementation dependency.

## Executive decision

No reviewed project should be the Alpine GPUI codebase.

Use:

- Zed GPUI for the proven entity, element, scene, native Metal, and headless-test
  concepts;
- GPUI-CE for platform crate separation and upstream-drift lessons;
- WGPUI and the `gpui-wgpu` lineage for unified-backend, embedded-surface, and
  event-loop failure cases;
- Kael for feature taxonomy, damage tracking, render-graph research, and broad
  conformance ideas;
- awesome-gpui and component libraries as downstream workload catalogs.

Implement Alpine GPUI independently with direct Metal ownership.

## Repositories reviewed

| Project | Reviewed commit | Role | Assessment |
| --- | --- | --- | --- |
| [Zed GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) | `1271f8b0` | Original production framework | Best proof of the core programming model and native Metal viability; coupled to Zed priorities and workspace evolution |
| [awesome-gpui](https://github.com/zed-industries/awesome-gpui) | `b22321c8` | Ecosystem catalog | Workload and API demand inventory, not framework source |
| [GPUI-CE](https://github.com/gpui-ce/gpui-ce) | `b172d695` | Community edition | Strongest backend-crate decomposition; complex upstream synchronization and dependency ownership remain unresolved |
| [mdeand/gpui-wgpu](https://github.com/mdeand/gpui-wgpu) | `a2158ca3` | WGPU plus winit fork | Useful unified-backend experiment; weak CI and unresolved redraw work |
| [nestrilabs/gpui-wgpu](https://github.com/nestrilabs/gpui-wgpu) | `49d46d31` | Fork of mdeand | Same lineage with a small point-in-time divergence, not an independent architecture |
| [WGPUI](https://github.com/Far-Beyond-Pulsar/WGPUI) | `fd087f64` | Active WGPU plus winit fork | Most active unified-backend specimen; current CI is too narrow to establish cross-platform correctness |
| [Kael](https://github.com/Augani/kael) | `4d67872d` | Independent GPUI descendant | Broadest feature and test ambition; very large, young surface and draft shader API make it unsuitable as a trusted base |
| [gpui-unofficial](https://github.com/iamnbutler/gpui-unofficial) | `76d3fc3b` | Automated release transform | Useful provenance and packaging lesson, not an architectural foundation |

## Zed GPUI

### Keep as a concept

- Hybrid immediate and retained view construction.
- Entity handles with context-mediated mutation and notification.
- Element request-layout, prepaint, and paint phases.
- Backend-neutral `Scene` consumed by platform windows.
- Native Metal renderer and sprite atlas specialization.
- Test platform, headless renderer, visual-test context, and input simulation.
- Virtualized lists and application-scale text workloads.

### Improve or reject

- GPUI is developed for Zed, so framework needs that do not help Zed can remain
  unsupported.
- The public core historically knows a broad platform trait surface and many
  product-adjacent services.
- Workspace-internal dependencies and release cadence complicate independent
  consumption.
- Compatibility with Zed's public API is not an Alpine GPUI goal.

## GPUI-CE

GPUI-CE has moved native backends into `gpui_macos`, `gpui_linux`,
`gpui_windows`, `gpui_wgpu`, and `gpui_platform`. This is directionally better
than embedding every backend in the public UI crate and is the most relevant
structural reference for Alpine GPUI.

Useful material:

- direct Metal remains the macOS default;
- Linux and web share WGPU rendering without requiring macOS to do so;
- Windows keeps DirectX while exposing optional WGPU integration;
- a headless Metal renderer supports renderer-backed visual tests;
- upstream synchronization scripts explicitly separate community-owned files.

Risks observed:

- [dependency ownership and git-only pins remain an explicit open problem](https://github.com/gpui-ce/gpui-ce/issues/74);
- [upstream manifest reconciliation is not yet mechanically reliable](https://github.com/gpui-ce/gpui-ce/issues/79);
- the inspector reserves space but does not render because the implementation
  remains coupled to Zed's application UI ([issue 93](https://github.com/gpui-ce/gpui-ce/issues/93));
- visual extensibility and custom shader gaps are driving ecosystem
  fragmentation ([issue 50](https://github.com/gpui-ce/gpui-ce/issues/50));
- security audit jobs currently use `continue-on-error`, and workflow actions
  are version-tagged rather than pinned to immutable commits;
- the reviewed `main` run failed its lockfile job because CI executed
  `cargo update` and treated newly available transitive releases as repository
  staleness. The actual Linux, macOS, Windows, Clippy, WASM, and example jobs in
  that run passed. Alpine GPUI will test the committed lockfile with `--locked`
  and keep dependency updates in dedicated changes.

Conclusion: model the crate boundaries and headless-render concept, not the
upstream synchronization model or dependency graph.

## The two `gpui-wgpu` links

These are one lineage. GitHub identifies Nestri as a fork of mdeand, which is
itself a fork sourced from GPUI-CE.

Both replace native windowing and rendering with winit and WGPU. Their useful
ideas include:

- a single portability implementation for behavioral comparison;
- an explicit WGPU context and surface registry;
- embedded WGPU surfaces for applications with their own GPU content;
- WGSL versions of the fixed GPUI primitive shaders;
- cosmic-text and swash as a portable text experiment.

The key warning is frame scheduling. The open mdeand performance PR reports an
implementation that redrew at every event-loop tick and consumed roughly 45%
CPU and 35% GPU while idle on an M2 MacBook Air. Its proposed demand-driven wake
path reduced observed idle use to approximately zero. The report is not a
controlled benchmark, but the failure mode is structurally credible and becomes
a mandatory Alpine GPUI scheduler test.

The mdeand main workflow was failing at the reviewed date because the Ubuntu
runner lacked a fontconfig system dependency. Nestri had no workflows. Neither
repository provides evidence for Metal-specific performance or cross-platform
release quality.

Conclusion: keep surface-registry and portability tests as specimens. Do not
inherit winit, WGPU, or their scheduling.

## WGPUI

WGPUI is a separate, active GPUI-CE fork pursuing the same unified WGPU plus
winit strategy. It adds useful embedded-surface examples and is a better ongoing
portability specimen than the smaller `gpui-wgpu` forks.

Its reviewed workflow runs only on Ubuntu. A green main run therefore proves
the Linux build and tests, not macOS Metal behavior, Windows behavior, Wayland
integration, or display scheduling. Its abstraction also deliberately removes
the direct native-backend control that Alpine GPUI requires.

Conclusion: use as a differential behavior oracle later, especially for
embedded surfaces and WASM. Do not make it a production dependency.

## Kael

Kael is the most ambitious independent descendant. It retains native Metal,
DirectX 11, and Blade/Vulkan paths and contains valuable work on:

- damage and frame skipping;
- renderer-backed headless output;
- GPU budgets and a backend-neutral render graph;
- gradients, transforms, effects, transitions, and virtualization;
- broad native service capability reporting;
- a component library and maintained showcase application;
- platform readiness workflows across macOS, Windows, X11, and Wayland.

The reviewed repository contains over half a million lines of Rust across more
than 700 Rust files and was created in May 2026. Its public custom shader design
is still marked draft, and the document states that the current primitive
renderer remains fixed-function. Breadth and green CI are useful evidence, but
they are not substitutes for maturity, focused performance characterization,
or an auditable understanding of every subsystem.

Conclusion: mine its test taxonomy, damage cases, capability model, and render
graph questions. Do not adopt its codebase or match its feature breadth before
the rendering and runtime kernel are stable.

## awesome-gpui and downstream libraries

awesome-gpui is useful because it reveals the real workload surface:

- editors, terminals, database clients, media tools, whiteboards, and large
  tables;
- `gpui-component` as a broad styled-component corpus;
- Base GPUI as a headless primitive API direction;
- Storybook-like tooling, routers, plotting, video, PDF, and custom canvases.

Alpine Lab should turn these into conformance categories. We should not promise
source compatibility with GPUI, but familiar concepts and a migration layer can
be considered after the core API proves itself.

## What Alpine GPUI will pick

### Adopt as concepts

- Entity-owned state with transactional mutation.
- Hybrid immediate and retained view construction.
- Immutable scene snapshots and backend-owned resources.
- Native Metal renderer with specialized batching and atlases.
- Test platform plus renderer-backed headless fixtures.
- Demand-driven scheduling with explicit wake and present states.
- Embedded custom GPU surfaces behind a safe resource-lifetime contract.
- Capability reports, damage tracking, GPU budgets, and diagnostics.
- Separate headless primitives and styled component layers.

### Defer behind measurements

- Taffy as the layout engine.
- A portable shader source and translation pipeline.
- A general render graph before fixed 2D primitives are efficient.
- WGPU as a compatibility backend.
- Cross-platform source compatibility with GPUI.

### Reject

- Wholesale forking or automated upstream merges.
- A generic GPU abstraction in the direct Metal hot path.
- Continuous redraw tied to the event-loop tick.
- Git dependencies in shipping manifests.
- Non-blocking security checks without an expiring exception.
- Unpinned CI actions.
- Exact cross-GPU pixel hashes as the only visual oracle.
- Shipping a broad native-service catalog before the framework kernel is
  measurable and reliable.

## Required tests derived from upstream failures

1. Idle windows submit no frames and consume no renderer work.
2. Multiple wake requests coalesce into one scheduled frame.
3. GPU surface resize, loss, and presentation cannot race resource destruction.
4. Lockfile validation never performs an implicit dependency update.
5. Inspector and diagnostic UI are built with public framework APIs.
6. Every backend supports offscreen readback before visual tests become gates.
7. Visual comparison separates semantic scene snapshots, CPU geometry oracles,
   and GPU pixel tolerances.
8. Upstream research never enters the product without a provenance entry.
