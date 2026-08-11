# Shipping Crate Instructions

These rules apply to every crate below `crates/`.

## Boundaries

- A crate owns one coherent responsibility and exposes the smallest useful API.
- Dependency direction follows `ARCHITECTURE.md`; no cycle or platform leakage
  is acceptable.
- Add no crate or dependency merely to reserve an architectural name.
- Public types need documentation, invariants, and failure behavior.
- Shipping crates may not depend on test applications or diagnostic UI.

## Safety and performance

- Safe crates deny unsafe code.
- FFI crates require a documented safe boundary and a safety invariant for
  every unsafe block.
- Avoid hidden heap allocation, dynamic dispatch, reference counting, locks,
  and clones in frame, layout, input, and text hot paths.
- Resource and cache growth must be measurable and bounded.
- Use typed identities and units where primitive confusion can violate safety or
  correctness.

## Tests

- Keep unit tests near the behavior they prove.
- Add integration tests for crate boundaries and replacement facades.
- Use property or model tests for geometry, ordering, scheduling, and lifetimes.
- Test invalid inputs and failure paths, not only successful examples.
- Run focused crate tests, then `scripts/check.sh`.

## Public API

- Do not stabilize an API before a real vertical slice consumes it.
- Prefer associated types and generics in hot paths when they preserve clarity.
- Use trait objects only where runtime heterogeneity is an actual requirement.
- Breaking changes are permitted before version 1 but still require a change
  fragment and migration note when an application can observe them.
