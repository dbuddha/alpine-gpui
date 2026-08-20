---
title: Native idle wakeup and energy qualification
status: active
reviewed: 2026-08-20
issue: 237
evidence_level: E3
---

# Native idle wakeup and energy qualification

This package defines how Alpine distinguishes a demand-driven native renderer from an application that merely appears visually idle. It supports Task #237 and the M2 native macOS exit gate.

The package is deliberately split between hosted structural evidence and physical fixed-hardware evidence. Hosted CI can prove Alpine's own admission, callback, frame-slot, and presentation counters remain unchanged while the main run loop advances. It cannot prove physical compositor occlusion, package energy, thermal stability, or cross-product superiority.

## Contents

- [Source map](source-map.md)
- [Findings](findings.md)
- [Experiments](experiments.md)
- [Decisions](decisions.md)
- [Fixed-hardware protocol](../../quality/native-idle-energy.md)

## Claim boundary

The strongest claim available before physical capture is: on the hosted Apple Silicon macOS configuration, the tested production AppKit states admitted no additional Alpine callback, Metal submission, or direct-present call after settlement, and an explicit invalidation control admitted one frame before returning to quiescence.

No hosted result authorizes a wakeup, power, physical occlusion, comparator, or universal framework claim.
