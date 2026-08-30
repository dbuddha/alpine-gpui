# Vision and mission

Alpine GPUI exists to make demanding native desktop applications feel immediate
while keeping ownership, failure, memory, accessibility, and performance
observable. It is an independent Rust implementation conceptually adapted from
Zed GPUI, with Apple Silicon and direct Metal as its first proving ground.

## Mission principles

- **MP-01, native excellence:** preserve direct control of Metal, Cocoa, input,
  text, accessibility, lifecycle, and measurement on Apple Silicon.
- **MP-02, explicit contracts:** make state transitions, ownership, allocation,
  synchronization, failure, and recovery visible in APIs and tests.
- **MP-03, qualified correctness:** match each claim to the evidence class that
  can actually test or prove it, and state bounds and remaining uncertainty.
- **MP-04, traceable delivery:** connect user outcomes and research to approved
  claims, implementation, CI evidence, successful revisions, and releases.
- **MP-05, measured efficiency:** do no idle rendering, bound retained work, and
  qualify latency, throughput, energy, allocation, and memory on real hardware.
- **MP-06, accessible interaction:** treat semantics, focus, keyboard operation,
  IME, announcements, and platform accessibility as framework contracts.
- **MP-07, editor-specialized rendering:** combine a proper native graphical
  shell with a visible-range text canvas, purpose-built layout, bounded GPU
  resources, and demand-driven Direct Metal presentation. The
  [editor rendering doctrine](concepts/editor-rendering-doctrine.md) owns the
  stable boundary and its evidence gates.

## Intended applications

The initial workload family includes code and text editors, terminals, database
clients, large data tables, inspectors, media surfaces, and multi-window
productivity tools. These are case-study and dogfood targets, not a promise that
every application class is implemented today.

## Boundaries

Alpine owns its runtime, scene protocol, native renderer policy, resource
lifecycle, platform integration, testing surface, and application-ready
components. Version 1 does not target web, mobile, Intel macOS, GPUI source
compatibility, or a generic GPU abstraction in the direct Metal hot path.
