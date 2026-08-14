# Alpine Enhancement Proposals

An AEP is required when a Capability needs substantial ownership, state,
lifecycle, cross-platform, accessibility, performance, or formal-design work.
Small changes remain entirely in a GitHub Requirement.

Every major AEP defines stable atomic claim IDs, maps meaningful transition
properties into TLA+, identifies Rust and native boundaries, and specifies the
evidence required before its Capability can complete. An AEP is reviewed before
its Capability and Requirements receive `owner:approved`. The initial AEPs are
bootstrap exceptions authorized directly by the owner's approved plan.

An accepted AEP is historical. Later design creates a superseding AEP and links
the old one instead of rewriting the original rationale. Current implemented
truth always moves into `ARCHITECTURE.md` and rustdoc.

Accepted AEPs:

- [AEP 0009: Multi-layer assurance](0009-multi-layer-assurance.md)
- [AEP 0016: Portable value contracts](0016-portable-value-contracts.md)
- [AEP 0025: Direct Metal offscreen renderer](0025-direct-metal-offscreen.md)
- [AEP 0028: Zed golden qualification](0028-zed-golden-qualification.md)

Required sections are motivation, journeys, goals, non-goals, atomic claims,
model, Rust ownership, correctness, accessibility, performance, memory, failure,
platform scope, evidence, mapping, risks, and reversal conditions. If no
meaningful states, actions, invariants, or progress properties exist, decompose
the work or keep it issue-only rather than creating a ceremonial model.
