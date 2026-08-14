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

The next full application is the unapproved
[Direct Metal offscreen Capability #25](https://github.com/dbuddha/alpine-gpui/issues/25).
Its AEP must define resource and device-loss lifecycle states, native ownership,
readback oracles, and fixed-hardware budgets before Requirements are approved.

Required sections are motivation, journeys, goals, non-goals, atomic claims,
model, Rust ownership, correctness, accessibility, performance, memory, failure,
platform scope, evidence, mapping, risks, and reversal conditions. If no
meaningful states, actions, invariants, or progress properties exist, decompose
the work or keep it issue-only rather than creating a ceremonial model.
