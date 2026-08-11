# ADR 0002: Adopt the version 1 product contract

- Status: Accepted
- Date: 2026-08-10

## Context

The complete framework goal left several choices open: compatibility with GPUI,
first workload, component breadth, styling, layout and text providers, custom
GPU content, platform appearance, dependency boundary, licenses,
accessibility, dogfood applications, and performance enforcement.

## Decision

Alpine GPUI will:

- use familiar GPUI concepts without source compatibility;
- optimize first for data-heavy productivity applications;
- deliver essential components through vertical slices;
- use typed Rust styling and theme tokens without a CSS runtime;
- isolate layout behind an Alpine facade and treat Taffy as an oracle or
  temporary provider candidate;
- implement CoreText first behind a portable text contract;
- make embedded Metal surfaces and custom materials first-class;
- adapt appearance and behavior to each desktop platform;
- target desktop only through version 1;
- allow audited bindings and standards-heavy Rust libraries behind Alpine-owned
  policy and hot paths;
- allow permissive shipping licenses by default and require explicit exceptions;
- require accessibility from the first interactive component implementation;
- dogfood through Alpine Lab and Alpine Workspace;
- define aggressive provisional performance targets and calibrate them on fixed
  M1-class hardware.

Existing decisions remain: private and proprietary, Rust implementation, macOS
15 minimum, Apple Silicon only, direct Metal first, direct Vulkan and D3D12
later, and CI spending below the owner-approved monthly cap.

## Consequences

- Source compatibility and broad component parity cannot delay the kernel.
- Runtime and layout precede full text and component integration.
- Native behavior may differ when portable semantics remain consistent.
- Accessibility and performance evidence are design inputs, not release polish.
- Dependency convenience cannot transfer scheduling, lifetime, or renderer
  policy outside Alpine.
