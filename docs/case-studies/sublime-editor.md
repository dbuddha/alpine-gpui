# Sublime Text local-first performance model

- Reviewed: 2026-08-15
- Research: [#114](https://github.com/dbuddha/alpine-gpui/issues/114)
- Influence: local-first editor speed and memory posture

## Scope

The review captures patterns from Sublime's local-only editor behavior where
startup, typing, scrolling, and large-file workflows are optimized around the
single-machine user path. This is a policy and architecture study only and does
not include proprietary implementation transfer.

## What to copy for Alpine

- **Deterministic startup cost control**
  - Defer non-essential services until a meaningful local feature request arrives.
  - Keep first surface materialization minimal so first-key responsiveness is
    predictable.
- **Demand-driven paint updates**
  - Coalesce bursts of mutation into limited redraw turns instead of continuous
    polling.
  - Prefer latest-wins frame admission for rapid typematic and gesture sequences.
- **Input-first scheduling**
  - Treat input and accessibility as first-class turn boundaries.
  - Preserve IME and key-repeat behavior across visibility, focus, and recovery.
- **Large-file safety**
  - Use viewport-centered rendering and virtualization for long files so edit
    operations remain bounded under deep scroll.
  - Use capped caches with deterministic eviction when memory pressure is
    detected.
- **Memory residency posture**
  - Keep parse/index/language and scene caches bounded.
  - Separate transient hot-path allocations from long-hold buffers.
  - Drain and measure shutdown state to avoid hidden retention growth.

## What not to copy

- Proprietary text engine or internal parser implementation.
- Exact theme/command or visual language decisions.
- Proprietary plugin API contracts and binary formats.
- Any cloud workflow for collaboration, AI, remote account, or diagnostics.

## Performance and memory guidance for Alpine

1. Startup should remain fast even before language or extension services fully
   initialize.
2. Input to render scheduling should prefer short deterministic turns with explicit
   backpressure behavior.
3. Virtualization and viewport windows should be visible as first-class
   architecture, not a later optimization.
4. Memory growth should be bounded by explicit caps and measured under
   long-run edit and scroll so retention is visible to qualification gates.
5. Recovery paths (close, shutdown, hidden/visible, sleep/wake) should never
   leak resources into the next run window.

## Exclusion boundaries for this v1

- No direct collaboration workflows.
- No hosted model or cloud AI integrations.
- No remote editing or remote compute dependency.
- No public extension marketplace parity.
- No telemetry collection and no business analytics instrumentation.
- No debugger integration and no debugger-driven workflow claims.

## Evidence output we keep as first-class

- Startup-to-first-edit latency with fixed workload and environment identity.
- Continuous text editing latency under burst and coalesced frame scheduling.
- Large-file scroll and local symbol lookup behavior under memory pressure.
- Input-to-photon latency for IME and focus transitions.
- Peak and post-shutdown resident memory traces.
