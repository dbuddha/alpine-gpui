# Dependency Policy

The initial workspace intentionally has no third-party Rust dependencies.

## Admission criteria

An external dependency must have:

- a narrow responsibility that is expensive or risky to reproduce;
- a compatible license and recorded provenance;
- active maintenance or a credible vendoring and patch strategy;
- bounded transitive dependencies with default features disabled where useful;
- no hidden runtime, allocator, thread, or GPU ownership policy;
- locked versions and a deterministic update process;
- tests at our facade boundary so the implementation can be replaced.

Git dependencies are prohibited in production manifests. Temporary research
tools must not enter the shipping dependency graph.

## Proposed dependency batches

No batch is approved merely by appearing here.

### Metal and macOS bindings

Candidate role: generated Objective-C and Metal API bindings only.

- `objc2`
- `objc2-foundation`
- `objc2-app-kit`
- `objc2-metal`
- `objc2-quartz-core`

These crates would expose native APIs. Alpine GPUI would still own application,
window, layer, renderer, resource, and scheduling policy.

### Correctness and developer tooling

- property testing for geometry and state machines;
- benchmark harness and statistical reporting;
- license, advisory, and duplicate-dependency checks.

These are development-only and must be pinned independently of product code.

### Text and standards

Unicode shaping, bidi, segmentation, line breaking, font parsing, and
accessibility are standards-heavy areas where a reviewed pure Rust dependency
can be safer than a new implementation. Each capability remains behind a Rock
GPUI facade and receives its own conformance corpus.

### Layout

Taffy is a candidate oracle or initial implementation behind a facade, not an
architectural dependency. We will decide after measuring its invalidation,
allocation, grid, and pathological-layout behavior against our target workloads.
