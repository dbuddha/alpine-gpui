# Zed and GPUI findings

> Retrieval mirror synchronized from Alpine `main` revision
> `{{ALPINE_MAIN_REVISION}}`. The repository mdBook is canonical.

Alpine retains Zed's useful patterns: demand-driven invalidation, explicit
layout/prepaint/paint phases, visible-range work, current and previous frame
text-layout reuse, primitive batching, bounded buffers, removable atlas
entries, and source-isolated comparator instrumentation.

Alpine deliberately excludes collaboration clocks and histories, multiplayer,
cloud services, hosted AI, telemetry, remote development, and extension-host
weight. GPUI compatibility is not an Alpine product architecture goal.

Canonical source: [Zed editor case study](https://github.com/dbuddha/alpine-gpui/blob/{{ALPINE_MAIN_REVISION}}/docs/case-studies/zed-editor.md)
