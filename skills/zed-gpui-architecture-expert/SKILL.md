---
name: zed-gpui-architecture-expert
description: Dissect pinned Zed Editor and GPUI source, map behavior and ownership to Alpine, and justify narrow adaptations with provenance and equivalent evidence.
---

# Zed and GPUI architecture translation

Read the current comparator pin and [lineage source map](../../docs/research/alpine-lineage/source-map.md)
before inspection. Keep the accepted comparator revision separate from newer
upstream review revisions. Never silently use `latest` documentation as the pinned
implementation. Start from one editor behavior or measured renderer question.

## Source investigation

Trace input through ownership, mutation, invalidation, layout/prepaint/paint,
scene lowering, native submission and completion. For editor work, include text
buffer snapshots, selection/undo, multi-buffer semantics, language-server document
ownership, syntax invalidation, workspace and accessibility. Read callers, failure
paths and tests, not just an attractive struct or a blog diagram.

Distinguish observed source behavior, an author's stated rationale, Alpine's
inference and locally measured evidence. Link exact commits, paths and ranges.
Use [inspection routes](references/inspection-routes.md) to find the relevant
modules, then confirm locations at the chosen revision.

## Translate only the useful mechanism

Retain useful patterns such as visible-range construction, demand invalidation,
frame-local element phases, line-layout reuse, glyph admission, batching and
bounded GPU buffers when the Alpine workload consumes them. Do not port GPUI's
entity graph, registries or compatibility surface solely for resemblance.

Exclude collaboration clocks, remote operations, AI/accounts, telemetry, plugin
hosting and other rejected product scope. Removing a feature does not prove lower
latency or memory: inspect what work/resources actually disappear. Preserve the
accepted local editing, IME, accessibility and durability semantics.

Zed application source and instrumentation stay in the isolated GPL lab. GPUI has
its own licensing boundary; verify it at the pin. Conceptual translation is not
copied code. Any source copying needs precise provenance, applicable license and
review; never launder copied application code through a paraphrased patch.

## Comparison and historical record

Use identical validated trace semantics and timer endpoints. Separate adaptation,
framework scene construction, renderer submission/readback, GPU work and product
journeys. Retain the miniature baseline without calling it representative or
replacing an unfavorable result with an easier endpoint.

For each change record origin, Alpine destination, transformation, include/exclude
decision, correctness evidence, performance and memory results independently,
limitations, issue/PR and accepted revision in the existing lineage package.
Report narrower wins honestly; no percentage of "GPUI adapted" without a defined
mechanism inventory and denominator. Route hardware interpretation to
`$apple-metal-performance-engineer`, and editorial publication to
`$github-documentation-architect`. WGPU and community GPUI projects remain source
or comparator inputs, not automatically approved dependencies.
