---
scope: framework crates under crates/
parent: ../AGENTS.md
updated: 2026-10-02
known_defects: []
---

# Framework internals

Rules for changes under `crates/`. The root AGENTS.md owns the invariants and
budgets; this file owns how the framework keeps them.

## Crate map

| Crate | Owns |
| --- | --- |
| `alpine-core` | Validated geometry and value contracts |
| `alpine-scene`, `alpine-renderer` | Immutable scenes, painter order, renderer contracts |
| `alpine-text`, `alpine-text-layout` | Text revisions, Unicode coordinates, layout and glyph caches |
| `alpine-runtime` | App events, bounded workers, stale-result admission |
| `alpine-platform` | Presentation state machine and frame slots |
| `alpine-metal`, `alpine-platform-macos` | GPU ownership, AppKit callbacks, presentation, teardown |

## Presentation

- The display link starts paused, resumes only for visible dirty work and
  pauses when clean. Never acquire a drawable or encode while hidden,
  minimized, zero-size or occluded: `nextDrawable` stalls there.
- The display-link callback encodes, commits, presents and returns. No
  `waitUntilCompleted` outside offscreen readback, no `presentAtTime` with
  CAMetalDisplayLink, no fourth command buffer when the three slots are busy.
- Presentation telemetry never owns slot release.
- The always-on recorder (`recorder.rs`) is the one sanctioned global:
  main-thread diagnostic state started by the first surface, holding no app
  state. Its rings are reserved once, never reallocated, with no timer.
- In-flight frames and closes finish without display-link updates: a 40 ms
  frame-progress timer, alive only while a frame is in flight, polls
  completion, falls back once the link is silent and drains a close.
  Drawables are released at GPU completion.
- Each surface owns its revisions, epochs, display link and slots. One blocked
  or occluded surface must not stall another (panes, windows, later the
  terminal).
- Device loss invalidates the backend generation; nothing recreates it today.

## Text and atlas

- A warm, unchanged frame does no glyph rasterization, atlas publication or
  upload. Look up before rasterizing.
- A layout fingerprint is only a filter; exact range equality is the guard.
- Use grid arithmetic where it is correct, but never trade pixel scrolling,
  shaping, graphemes, font fallback, bidi, hit testing or accessibility for a
  fixed-cell shortcut.

## Accessibility

Accessibility is part of every interactive component's contract. Semantics stay
separate from visual primitives. AppKit pulls on the main thread; snapshots
carry no document text, which is pulled by exact revision in bounded ranges. A
query never causes a frame. Post notifications only after RefCell borrows end,
and destroy elements before revoking the handler.

## Safety and lifecycle

- Unsafe code lives only in the files `scripts/check-policy.sh` lists, each
  with a local safety argument and focused tests.
- Watch lock and RefCell reentrancy across native callbacks, and main-thread
  blocking. Native handles, callback generations and teardown need explicit
  owners.
- Preserve blended painter order. A pixel hash alone proves nothing; use
  semantic and readback checks.
- The release profile is `panic = "abort"`: `catch_unwind` protects only debug
  and test builds.
- Never mutate the process environment after threads exist.
- Never invalidate a display link during a window close: QuartzCore's
  display-change callback can race `invalidate()` (#533). The close pauses
  and detaches the link; owner teardown invalidates it once.
- Metal validation on for correctness runs, off for timing runs.

## Native tests

AppKit tests are `harness = false` main-thread executables that need
`--cfg alpine_native_validation`; without it 13 of the 14 print a `skipped:`
line and pass. `native_surface` has no such gate and shows a real window, so
`scripts/check.sh` opens one. A test filter
matching zero tests exits 0, so check the count. Hosted CI uses the test-only
Metal route and `ALPINE_PRESENTATION_EVIDENCE_MODE=hosted-direct`.

## Technique boundary

Admitted: triple buffering, reusable upload memory, batching, offline built-in
assets, GPU profiling. Rejected: GPUI's entity graph and global registries,
tokio, WGPU/Naga/WGSL in shipping code, render graphs, ECS, CSS or flexbox
layout, generalized animation, continuous game loops, MetalFX.

## Commands

```sh
cargo run --locked -p alpine-editor
cargo test --locked -p <crate>
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo fmt --all -- --check
scripts/check.sh
scripts/check-native.sh physical shipping
scripts/build-alpine-editor-app.sh
scripts/launch-alpine-editor-app.sh <file-or-folder>
```
