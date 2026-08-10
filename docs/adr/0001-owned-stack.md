# ADR 0001: Build an owned stack from architectural specimens

- Status: Accepted
- Date: 2026-08-09

## Decision

Alpine GPUI will not fork GPUI, GPUI-CE, WGPUI, a `gpui-wgpu` repository, or
Kael as its product foundation.

We will implement owned boundaries and use those projects as architectural,
behavioral, performance, and test-corpus specimens. Limited Apache-2.0 code may
be incorporated only after a source-level review and a provenance entry.

## Rationale

A wholesale fork would immediately inherit upstream coupling, dependency pins,
platform policy, API compatibility obligations, and existing defects. A new
implementation gives us control over frame scheduling, renderer ownership,
error behavior, testability, and platform-specific fast paths.

Starting without specimens would discard years of practical learning. Studying
multiple independent branches exposes which patterns survive different product
requirements and which failures recur.

## Consequences

- Initial feature delivery is slower than renaming a fork.
- Architecture and acceptance tests must precede large implementation slices.
- Upstream changes are reviewed intentionally, not merged automatically.
- Provenance must remain auditable even though the repository is private.
- WGPU can be used later for differential tests without controlling Metal.
