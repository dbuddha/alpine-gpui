# AEP 0016: Portable value contracts

- Status: accepted bootstrap
- Capability: [#16](https://github.com/dbuddha/alpine-gpui/issues/16)
- Requirement: [#17](https://github.com/dbuddha/alpine-gpui/issues/17)
- Mission: MP-02 and MP-03
- Motivating findings: CS-ZED-002 and CS-WGPU-002

## Motivation and journey

Every later scene, layout, renderer, input, and accessibility system relies on
geometry and color values that reject invalid states consistently. A caller
submits raw scalars, constructors admit or reject them, and downstream code can
rely on the accepted contract without backend-specific validation.

## Goals and non-goals

Specify and qualify finite points, non-negative finite sizes, normalized linear
RGBA channels, and positive-area rectangle intersections. This AEP does not
specify transforms, clipping trees, layout, color spaces, GPU conversion,
allocation, concurrency, or elapsed-time performance.

## Atomic claims

- **AEP-0016-C01:** `Size::new` accepts every bounded non-negative finite pair,
  preserves both values, and reports empty exactly when an extent is zero.
- **AEP-0016-C02:** Byte-normalized RGBA channels always produce a valid
  `LinearRgba`, including zero and one endpoints.
- **AEP-0016-C03:** Every returned intersection in the bounded proof domain has
  positive extent and remains contained by both input rectangles.

## TLA+ model

[`PortableValues.tla`](../../formal/tla/aep-0016/PortableValues.tla) models raw
value selection and admission against a finite valid set. `AcceptedIsValid` is
the safety invariant and `CanAccept` establishes reachability. The model maps to
claims C01 and C02. `Faulty.cfg` permits invalid admission and must produce a
counterexample. Intersection arithmetic remains implementation evidence in
Kani and dynamic tests rather than being ceremonially duplicated in TLA+.

## Rust ownership and state transitions

`alpine-core` owns the values. Constructors return `Option` and allocate
nothing. `Rect::intersection` consumes copied values and returns a copied
result. There are no native handles, shared owners, asynchronous transitions,
or unsafe boundaries.

## Correctness, performance, memory, and accessibility

Kani proofs use exact symbolic byte and unsigned-short domains, state every
assumption, and cover valid and boundary paths. Dynamic tests cover invalid
floating-point values and serve as regression playback. Constructors are
constant-space and allocation-free by inspection, but this AEP makes no timing
or process-memory claim. Logical geometry will later be consumed by semantic
and accessibility systems without a separate coordinate contract.

## Failure and recovery

Invalid input returns `None`; callers retain policy. Empty contact returns no
intersection. Proof failure produces a concrete counterexample that must become
a dynamic regression before the defect closes. New assumptions require review
and cannot be added merely to suppress a valid counterexample.

## Evidence and model-to-implementation mapping

| Abstract action | Rust boundary | Evidence |
| --- | --- | --- |
| `Choose` | caller supplies scalar values | unit invalid-input cases |
| `Accept` | `Size::new` or `LinearRgba::new` returns `Some` | Kani plus dynamic companion |
| `Reject` | constructor returns `None` | unit tests and model conformance |
| bounded containment | `Rect::intersection` returns `Some` | Kani plus property-style dynamic cases |

TLA+ establishes a finite admission design. Kani checks compiled Rust over the
declared domains. Unit tests cover representative floating-point failures. No
formal refinement between the two is claimed.

## Risks and reversal conditions

The bounded domains do not cover arbitrary IEEE-754 behavior, overflow outside
the domain, native coordinate conversion, or transformations. Expand proofs or
adopt a more mathematical method only when a new Requirement states the larger
domain and its implementation mapping.
