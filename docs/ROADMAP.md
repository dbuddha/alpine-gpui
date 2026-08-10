# Roadmap

Milestones are gated by evidence, not dates.

## M0: Foundation

- Architecture, provenance, dependency, and CI policies
- Dependency-free core, scene, and renderer seams
- Three-OS compile and test matrix
- Managed Metal runner qualification specification

Exit gate: every committed file is reproducible from the repository and the
full baseline gate passes on Linux, macOS ARM64, and Windows.

## M1: Metal offscreen kernel

- Approved native binding dependencies
- Device and capability enumeration
- Command queue, pipeline, buffer, texture, and lifetime ownership
- Quad clear and readback without a visible window
- Validation-enabled negative and device-loss tests
- Golden image harness with exact and perceptual comparison modes

Exit gate: deterministic offscreen fixtures pass on a qualified Apple Silicon
GPU runner with zero Metal validation findings.

## M2: Native macOS presentation

- AppKit lifecycle and window creation in Rust
- CAMetalLayer ownership and resize handling
- Display link, frame coalescing, and occlusion behavior
- Retina scaling, color space, input, and clipboard foundations

Exit gate: a visible laboratory window remains idle when unchanged and presents
correctly at 60 Hz and 120 Hz without unbounded allocations.

## M3: Text, input, and accessibility

- Font discovery, shaping, fallback, rasterization, and atlas policy
- IME, selection, clipboard, bidi, CJK, emoji, and line breaking
- Focus graph and AccessKit-backed semantic tree

Exit gate: the text and accessibility conformance corpus passes with bounded
memory and deterministic semantic snapshots.

## M4: Runtime and UI system

- Entity state and transaction model
- Dependency tracking and scoped invalidation
- Element lifecycle, layout, hit testing, styling, animation, and virtualization
- Headless primitive layer separate from styled components

Exit gate: one million logical rows remain memory-proportional to the visible
window, and unchanged subtrees perform no layout or paint work.

## M5: Dogfood applications

- Alpine Lab for conformance fixtures
- Alpine Inspector for frame, scene, allocation, and GPU diagnostics
- Workspace stress application combining editor, table, canvas, terminal grid,
  docking, multi-window behavior, and accessibility

Exit gate: the profiler and inspector are built with the public framework API.

## M6: Additional platforms

- Direct Vulkan with Wayland first, then X11 compatibility
- Direct D3D12 with Win32
- Optional WGPU oracle backend for differential testing

Exit gate: shared conformance scenes and semantic tests pass across backends,
with platform-specific tolerances and capability manifests.
