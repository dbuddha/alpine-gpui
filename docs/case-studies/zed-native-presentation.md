# Zed native macOS presentation at v1.15.0

- Snapshot date: 2026-08-14
- Repository: <https://github.com/zed-industries/zed>
- Stable tag: `v1.15.0`
- Exact commit: `e17dc4f9d50db73a458b64dcce50ecd4878b98a3`
- Application license: GPL-3.0-or-later
- GPUI and `gpui_macos` license: Apache-2.0
- Research record: [#65](https://github.com/dbuddha/alpine-gpui/issues/65)
- Influence: conceptual, behavioral, workload-based, and validation-oriented

## Scope

This snapshot reviews `crates/gpui_macos/src/display_link.rs`,
`metal_renderer.rs`, and `window.rs` at the exact revision above. It focuses on
native frame pacing, layer configuration, visibility, resize, scale, drawable
ownership, presentation, and teardown. No Zed source or asset is copied into
Alpine.

## Durable findings

- **CS-ZED-PRESENT-001:** Zed treats display pacing as a visibility-scoped
  subscription, not as a permanent animation loop. Frame notifications are
  coalesced onto the main queue.
- **CS-ZED-PRESENT-002:** Zed's `CVDisplayLink` teardown history demonstrates
  that native callback lifetime is a correctness boundary. Its pinned design
  uses one immortal link per display to avoid use-after-free races.
- **CS-ZED-PRESENT-003:** Window size, backing scale, display migration, and
  occlusion change renderer state and frame scheduling. Treating them as
  cosmetic events would make stale or wasted work invisible.
- **CS-ZED-PRESENT-004:** Zed uses the supported three-drawable upper bound and
  keeps framebuffer relaxation test-only, but disables drawable timeout and
  therefore accepts an indefinite wait risk.
- **CS-ZED-PRESENT-005:** Transaction presentation during selected resize and
  activation paths shows that window lifecycle can change presentation
  semantics. Alpine needs explicit native tests before adopting any equivalent
  special path.
- **CS-ZED-PRESENT-006:** The pinned renderer selects `BGRA8Unorm` without an
  explicit layer color-space assignment in the reviewed path. Alpine must
  qualify its own transfer function and color-management contract rather than
  infer onscreen correctness from Zed's choice.

## Alpine decisions and rejected patterns

Alpine preserves the demand-driven visibility lesson, explicit size and scale
updates, bounded frame resources, and native lifecycle testing. Because Alpine
supports macOS 15 or newer, it selects layer-bound `CAMetalDisplayLink` rather
than reproducing the pinned Zed `CVDisplayLink` registry. Alpine retains
drawable timeout, keeps production drawables framebuffer-only, and uses its
offscreen renderer for pixel readback.

Rejected patterns are immortal display-link objects, indefinite drawable
waits, a permanent animation loop, reading mutable application state during
native encoding, qualifying a stale surface epoch, and transferring Zed
implementation code into Alpine.

## Derived Alpine claims

AEP 0064 derives C01 through C08 from this snapshot and Apple platform
research. Requirement #67 starts the single-window lifecycle. Later
multi-window, input, accessibility, and product-journey Requirements must cite
their own observable behavior and evidence rather than treating this case
study as verification.

This document remains a description of the pinned revision. Weekly upstream
radar may open new research, but it does not silently rewrite this snapshot.
