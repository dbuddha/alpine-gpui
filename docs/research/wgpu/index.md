---
title: WGPU deep research package
status: source-verified
reviewed: 2026-08-18
reviewed_revision: 8ee190c6f151c731a4f8cfd9a102d6ee5903460a
historical_revision: ee5cfb074fd0c4e318b5f8608df504678e4e17ac
release_context: v30.0.0
current_release_revision: 40f4a34ebaf56f9a046231f54125ad046239d3f3
current_release_context: v30.0.1
owner_requirement: 132
research_issue: 99
upstream_research_issue: 302
task: 202
---

# WGPU deep research package

This package retains the evidence behind Alpine's WGPU decisions. It is
designed for retrieval, challenge, and later requalification rather than as a
promotional overview.

## Research question

Which WGPU architecture, lifecycle, validation, testing, submission, and memory
patterns should affect Alpine GPUI, and which would add work that does not help
the Apple-first Alpine Studio daily-driver goal?

## Answer in one paragraph

Alpine should copy WGPU's discipline around layered safety contracts,
completion-owned resource lifetimes, structured surface outcomes, reusable
staging, no-GPU validation, real-GPU behavior tests, tolerant image comparison,
and explicit dependency testing. Alpine should not copy WGPU's portability
layers, WebGPU conformance surface, shader translation stack, generalized
resource-state tracking, remote registries, or broad backend matrix into v1.
WGPU remains a research specimen and a candidate differential oracle, not a
shipping dependency or substitute for direct Metal.

## Evidence status

| Class | Status |
| --- | --- |
| Primary source inspection | Complete at reviewed revision |
| Historical-to-current delta | Complete for Alpine-relevant paths |
| Official release review | Complete for v30.0.0 |
| Current stable delta | Complete for v30.0.1 with release-branch topology separated from upstream main |
| Alpine design inference | Recorded separately from source facts |
| Differential correctness experiment | Designed, not run |
| Performance or memory comparison | Not run; no claim permitted |
| Shipping dependency decision | Explicitly not approved |

## Package map

| Artifact | Purpose |
| --- | --- |
| [Source map](source-map.md) | Exact revisions, paths, source classes, and limitations |
| [Findings](findings.md) | Detailed correctness, performance, memory, and delivery analysis |
| [Experiments](experiments.md) | Fair differential, lifecycle, and residency protocols |
| [Decisions](decisions.md) | Include now, investigate later, and reject for v1 |
| [Case-study synthesis](../../case-studies/wgpu.md) | Decision-facing summary for implementers |

## Provenance and update rule

Research #23 remains the historical baseline. Research #99 owns the accepted
package, Research #302 owns the v30.0.1 branch-topology delta, and Task #202
owns the retained package and roadmap reconciliation. Every source pin remains
immutable evidence after upstream advances.

A future update must create or reopen an upstream-research record, select a new
exact revision, record the delta from this pin, and state which findings changed.
Changing a pin without retaining the old identity is forbidden.

## Evidence classification

- **Primary source:** behavior or architecture visible in the pinned WGPU
  repository, official WGPU Wiki, official API documentation, or release notes.
- **Alpine inference:** an Alpine design conclusion derived from one or more
  primary sources. It is not a claim about WGPU's measured performance.
- **Unverified hypothesis:** a question requiring an experiment. It cannot
  authorize implementation or a comparative claim.
