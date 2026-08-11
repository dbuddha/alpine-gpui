# Milestone Roadmap

This is the compact milestone view. `MASTER_PLAN.md` contains implementation
order and detailed gates. Milestones are evidence-gated, not date-gated.

## M0: Governed foundation

- Architecture, product contract, provenance, dependency, CI, and agent policy
- Dependency-free core, scene, and renderer seams
- Protected pull request workflow and three-platform CI
- Change-fragment and release-changelog mechanism

Exit: local and hosted gates agree, durable decisions are discoverable, and no
unapproved dependency or source incorporation exists.

## M1: Direct Metal offscreen kernel

- Approved native binding dependencies
- Device, queue, pipeline, resource, submission, and teardown ownership
- Offscreen primitives, readback, validation, and failure injection
- Golden image harness and observable work counters

Exit: deterministic fixtures pass on qualified Apple Silicon with zero Metal
validation findings and bounded resource growth.

## M2: Native macOS presentation

- AppKit lifecycle, windows, CAMetalLayer, displays, scale, and color
- Demand-driven display clock and frame coalescing
- Occlusion, resize, multi-window, input, and clipboard foundations
- Embedded Metal surfaces and custom materials

Exit: settled windows remain idle and 60 Hz or 120 Hz presentation has bounded
allocation, correct teardown, and no redundant submissions.

## M3: Runtime, layout, and events

- Entities, transactions, subscriptions, tasks, and scoped invalidation
- Element lifecycle, provider-isolated layout, hit testing, focus, and commands
- Animation scheduling and virtualized collections

Exit: unchanged subtrees do no work, ownership teardown is deterministic, and a
million logical rows remain proportional to the visible window.

## M4: Text, IME, and accessibility

- CoreText behind a portable Alpine contract
- Fallback, shaping, glyph cache, line breaking, selection, bidi, CJK, and emoji
- Marked text, IME, clipboard, semantic tree, and native accessibility bridge

Exit: the text, IME, and accessibility corpora pass with bounded memory and
deterministic semantic snapshots.

## M5: Components and dogfood

- Headless primitives separate from typed styled components
- Essential controls, overlays, text input, virtual lists, tables, trees, and docking
- Alpine Lab, Alpine Inspector, and Alpine Workspace

Exit: diagnostics use public APIs, every interactive component has accessibility
from inception, and the workspace meets calibrated performance budgets.

## M6: Additional desktop platforms

- Direct Vulkan with Wayland first, then X11
- Direct D3D12 with Win32
- Optional WGPU differential oracle

Exit: shared scene, event, semantic, and component suites pass with explicit
backend capabilities and tolerances.

## M7: Version 1

- Declared public API and compatibility policy
- Release candidate, migration, rollback, long-session, and fixed-hardware gates
- Signing, notarization, SBOM, and artifact attestations

Exit: the owner approves a fully reproducible, supported desktop framework
release.
