# AEP 0025 renderer lifecycle model

`RendererLifecycle.tla` models the first Direct Metal offscreen slice as one
renderer, one synchronous frame, and one exclusively owned frame resource.

- `renderer` maps to backend readiness, shutdown admission, and final teardown.
- `frame` maps to pure lowering, encoding, command submission, and terminal
  completion state.
- `resource` maps to the frame-local instance, target, and readback ownership
  token in the planned Rust transition core.
- `submitCount` maps to observable command-buffer commits.
- `releaseCount` maps to the one terminal release of frame-owned resources.
- `outcome` maps to the safe public success, error, or cancellation result.
- `BeginFrame`, `Encode`, `Submit`, `Complete`, `Fail`, and
  `CancelBeforeSubmit` map to explicit pure Rust transition functions and their
  native integration boundaries.
- `BeginShutdown` and `StopAfterDrain` map to admission closure, in-flight drain,
  and native object teardown.

The finite model has no unbounded collections and explores every state allowed
by the single-frame protocol. Weak fairness on the combined transition relation
supports the two progress properties. It assumes Metal eventually supplies a
terminal callback or wait result for committed work. The model excludes scene
contents, pointer and byte arithmetic, Objective-C retain behavior, actual GPU
execution, pixels, allocation failure details, device discovery, recovery into
a new backend generation, concurrency, and elapsed time.

Conformance requires Rust transition tests that replay every mapped action,
Kani for bounded state and arithmetic properties, and native tests for actual
Metal completion and teardown. These are separate evidence classes; no formal
refinement is claimed.

`Faulty.cfg` enables `ReuseInFlight`, which releases the resource while its
frame remains submitted. TLC must find an `InFlightOwnsResource` or
`FreeResourceIsInactive` counterexample.
