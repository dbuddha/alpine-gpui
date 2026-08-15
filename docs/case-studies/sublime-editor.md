# Sublime Text local-speed model

- Reviewed: 2026-08-15
- Research anchor: [#114](https://github.com/dbuddha/alpine-gpui/issues/114)
- Product: Sublime Text 4
- Source availability: proprietary implementation, public product documentation
- Influence: product philosophy, observable behavior, workload design
- Evidence strength: official public statements only

## Research question

Which publicly supportable Sublime techniques and product choices help a local
editor stay responsive, and which internal architectural claims must Alpine
refuse because the proprietary source is unavailable?

## Publicly supported facts

### SUBLIME-001: custom UI and GPU rendering

Sublime reports a rendering abstraction called a render context in its custom
UI framework and says widgets were moved onto its common primitives. Sublime
Text 4 uses OpenGL hardware acceleration, with macOS enabled by default
([Sublime HQ, 2021](https://www.sublimetext.com/blog/articles/hardware-accelerated-rendering)).

### SUBLIME-002: batching is a measured rendering technique, not a universal result

Sublime reports that batching glyphs changed full-frame time from 52 ms without
batching to 3 ms at its largest batch in one RX560 Linux 1440p experiment. The
same article warns that out-of-order glyph drawing can introduce rendering bugs
([Sublime HQ, 2021](https://www.sublimetext.com/blog/articles/hardware-accelerated-rendering)).

Alpine may copy the pattern of primitive-specific batching and independent
pixel oracles. It must not reuse those numbers as an Alpine target or claim,
because hardware, API, scene, application, and measurement boundaries differ.

### SUBLIME-003: indexing is low priority and configurable

Sublime documents low-priority background indexing processes, configurable
worker count, `.gitignore` exclusion, unknown-extension skipping, and explicit
status UI
([Sublime indexing documentation](https://www.sublimetext.com/docs/indexing.html)).

Alpine consequence: index work uses bounded low-priority workers, honors ignore
rules, exposes progress and degradation, and never blocks input or rendering.
This source does not disclose Sublime's in-memory index representation.

### SUBLIME-004: startup-critical work is deferred when possible

Sublime Text 4 reports asynchronous saving, faster handling of directories with
extreme file counts, lazy syntax embeds, faster syntax loading and matching,
and reduced syntax cache size
([Sublime Text 4](https://www.sublimetext.com/blog/articles/sublime-text-4)).

Alpine consequence: settings expansion, project indexing, non-visible syntax,
and session enrichment stay off startup-to-first-edit. Save remains atomic and
failure-visible even when write work is asynchronous.

## Alpine inferences, not Sublime facts

- A local editor benefits from a narrow process and service graph because fewer
  background owners simplify startup, shutdown, wakeups, and memory accounting.
- A purpose-built editor layout can be smaller than a browser-style layout and
  component system.
- A fixed built-in language cohort can avoid runtime plugin discovery and
  executable extension isolation.
- Degraded large-search behavior is preferable to unbounded result retention.

These are Alpine design conclusions derived from the product goal and public
behavior. They are not claims about Sublime's private implementation.

## Explicitly unknown

The public evidence does not establish Sublime's rope or piece-table design,
undo representation, line-layout cache, glyph-atlas eviction, allocator,
threading topology, frame scheduler, parse-tree residency, index data
structure, or exact startup dependency graph. Binary observation may measure
outcomes, but it cannot prove those internals.

Any future statement about one of these areas must be labeled as a hypothesis
and cannot justify an Alpine architecture decision without independent
evidence.

## Product philosophy adopted by Alpine Studio

- Open directly into useful local editing.
- Keep input, scrolling, save, search, and navigation predictable under load.
- Defer non-critical work and make background activity visible.
- Prefer built-in coherent behavior over ecosystem breadth.
- Keep idle work at zero and resource growth bounded.
- Degrade explicitly on huge inputs rather than hang or grow silently.

## Product policies not copied

- Sublime's plugin and package ecosystem is not included.
- Python runtime compatibility is not included.
- Cross-platform parity is not a v1 requirement.
- Sublime's proprietary implementation and commercial policies are not an
  Alpine source boundary.
- Exact UI or command compatibility is not a goal.

## Fair-comparison rules

Sublime is measured externally in safe mode with packages and plugins disabled.
The exact build, license mode, settings, font, syntax, repository fixture,
window geometry, display, OS, and workload are recorded. Stock-product
footprint is a separate result. No internal-stage timing is attributed to
Sublime unless an official API exposes that stage.

Product claims name the journey and endpoint, such as startup-to-accepted-key,
typing-to-presented-frame, project-search completion, steady physical footprint,
or post-close delta. A missing behavior invalidates the run.

## Sources

- [Faster Rendering Using Hardware Acceleration](https://www.sublimetext.com/blog/articles/hardware-accelerated-rendering): custom render context, OpenGL, batching, and published experiment scope.
- [Indexing documentation](https://www.sublimetext.com/docs/indexing.html): low-priority workers, exclusions, and progress.
- [Sublime Text 4](https://www.sublimetext.com/blog/articles/sublime-text-4): asynchronous save, lazy syntax embeds, cache and load-time improvements.
