# Alpine editor rendering doctrine

> Retrieval mirror synchronized from Alpine `main` revision
> `{{ALPINE_MAIN_REVISION}}`. The repository mdBook is canonical.

Alpine Studio is a full graphical native editor with terminal-like
implementation discipline, not a terminal emulator or fixed-cell TUI. Its v1
composition is an AppKit shell, a specialized visible-range text canvas,
purpose-built editor layout, immutable scenes, and demand-driven Direct Metal
presentation with zero idle submissions and bounded resource ownership.

The direction preserves full Unicode, IME, smooth scrolling, pointer input,
native accessibility, and lifecycle correctness. It adapts selected GPUI
principles without recreating GPUI's entity graph or compatibility surface.
WGPU remains a non-shipping comparator and differential-validation input.

Performance follows evidence, not visual style. The critical gates are:

- [Typing latency #304](https://github.com/dbuddha/alpine-gpui/issues/304) and
  [physical capture #331](https://github.com/dbuddha/alpine-gpui/issues/331).
- [Accessibility #253](https://github.com/dbuddha/alpine-gpui/issues/253) and
  [physical AX harness #273](https://github.com/dbuddha/alpine-gpui/issues/273).
- [Dogfood capture #238](https://github.com/dbuddha/alpine-gpui/issues/238)
  through [M5 acceptance #242](https://github.com/dbuddha/alpine-gpui/issues/242).
- [Realistic renderer traces #353](https://github.com/dbuddha/alpine-gpui/issues/353)
  and [E4 qualification #53](https://github.com/dbuddha/alpine-gpui/issues/53).

The doctrine rejects browser layout, continuous game loops, render graphs, ECS,
3D asset pipelines, generalized component infrastructure, and universal
performance claims without equivalent physical evidence.

- [Execution map](Execution-Map)
- [Implementation lineage](Research-Lineage)
- [Comparator qualification](Comparator-Qualification)

Canonical source: [editor rendering doctrine](https://github.com/dbuddha/alpine-gpui/blob/{{ALPINE_MAIN_REVISION}}/docs/concepts/editor-rendering-doctrine.md)
