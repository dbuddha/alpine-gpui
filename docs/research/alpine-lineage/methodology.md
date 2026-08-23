# Lineage and evidence methodology

## Taxonomy

Every mechanism uses exactly one primary lineage classification.

| Classification | Required proof | Allowed wording |
| --- | --- | --- |
| `ADAPTED-CONCEPT` | Pinned upstream source describes a mechanism and Alpine independently implements a materially related mechanism | "adapted the bounded-completion pattern" |
| `INDEPENDENT-CONVERGENCE` | Both systems meet the same OS, graphics, protocol, or editor requirement; no direct implementation derivation is established | "both use CoreText shaping" |
| `ALPINE-ORIGINAL` | No equivalent guarantee was found in the audited upstream boundary and Alpine has implementation evidence | "Alpine adds generation-safe terminal evidence" |
| `COMPARATOR-ONLY` | Source or implementation exists only in a lab, oracle, or workload definition | "WGPU is a differential oracle candidate" |
| `REJECTED` | The capability conflicts with Alpine's accepted product or architecture boundary | "the GPUI entity graph is intentionally absent" |
| `DEFERRED` | The capability may become useful after a named acceptance gate | "an element layer begins after sustained dogfood" |

`COPIED` is intentionally not a normal classification. It may be used only when
a code-level provenance audit identifies the exact source range, destination
range, license compatibility, transformation, author, and review approval.
This review found no basis to classify Alpine shipping code as copied from Zed
or GPUI.

## Evidence levels

Origin evidence and performance evidence are recorded independently.

| Level | Meaning |
| --- | --- |
| E0 Pointer | Unverified pointer, issue, catalog entry, or hypothesis |
| E1 Primary | Direct source code, official documentation, release, test, or measured raw artifact |
| E2 Triangulated | Multiple primary sources plus applicability and contradiction analysis |
| E3 Reproduced | Alpine locally reproduces the mechanism or measurement with retained identities and controls |
| E4 Qualified | Correctness-equivalent comparative evidence passes the accepted hardware, statistics, memory, and invalidation protocol |

Examples:

- A 10,000-frame unit regression can prove zero warm rasterization in that model
  at E3. It cannot prove lower real-world CPU usage than Zed.
- Exact cache byte accounting is implementation evidence. Physical footprint
  still needs allocator, OS, driver, GPU, and post-close measurements.
- A solid-quad readback comparison is E3 for that trace. It does not qualify
  glyph rendering, editor rendering, latency, or memory.

## Capability-family accounting

Capability counts are valid only when:

1. The family inventory is declared before drawing a percentage.
2. Each family is mutually exclusive in its primary classification.
3. Implemented, partial, deferred, rejected, and unqualified states remain distinct.
4. A count is never presented as LOC, effort, readiness, or performance.
5. Blocking correctness and latency gates override an unweighted completion count.

The inventory is maintained in [framework-lineage.md](framework-lineage.md) and
[studio-lineage.md](studio-lineage.md).

## Per-mechanism record

Every material entry in [evidence-ledger.md](evidence-ledger.md) records:

- Stable Alpine mechanism ID.
- Shipping Alpine symbol or path.
- Upstream source ID, or an explicit statement that no direct source applies.
- Lineage classification.
- What Alpine kept, changed, or excluded.
- Correctness evidence and current level.
- Performance or memory evidence and current level.
- Implementing issue or PR.
- Current claim status and the next experiment.

## Update workflow

Update this package in the same PR when any of the following occurs:

- A new shipping architecture or performance mechanism is introduced.
- A mechanism changes its ownership, bounds, caching, scheduling, or failure behavior.
- An upstream GPUI, Zed, WGPU, or awesome-gpui review changes an accepted conclusion.
- A new E3 reproduction or E4 qualification is retained.
- A mechanism is superseded, reverted, or removed.
- A milestone or issue state changes the current critical path materially.

The author performs this sequence:

1. Pin source and Alpine revisions in [source-map.md](source-map.md).
2. Add or update the capability row.
3. Update the mechanism row without overwriting old evidence.
4. Append a dated event to [history.md](history.md).
5. Record contradictions, invalid runs, and regressions rather than deleting them.
6. Link the implementation PR, issue, raw evidence, workload hash, and environment hash.
7. Advance an evidence level only when its stated gate passes.
8. Update the current verdict and Wiki retrieval summary if the critical path changes.

## Upstream review policy

The comparator pin and current-upstream review are separate lanes.

- Zed `v1.15.0` remains the immutable comparator until Requirement
  [#40](https://github.com/dbuddha/alpine-gpui/issues/40) accepts a requalification.
- Current Zed stable is reviewed under [#95](https://github.com/dbuddha/alpine-gpui/issues/95)
  and [#96](https://github.com/dbuddha/alpine-gpui/issues/96) without silently
  changing comparator artifacts.
- WGPU review changes are tracked under [#302](https://github.com/dbuddha/alpine-gpui/issues/302).
- awesome-gpui catalog drift is tracked under [#100](https://github.com/dbuddha/alpine-gpui/issues/100) and remains workload discovery, not architecture evidence.

## Claim grammar

Use:

- "avoids rasterization after warm admission in the deterministic regression"
- "retains at most three Alpine-owned presentation slots"
- "E4 comparison pending"
- "for workload X on environment Y at revisions A and B"

Do not use:

- "faster" from code inspection
- "memory efficient" from cache caps alone
- "120 FPS" without active-workload presentation evidence
- "copied from Zed" for a common graphics or editor pattern
- "feature parity" when excluded Zed subsystems are counted

## Review ownership

Repository Markdown and mdBook are canonical. GitHub Issues own live research
state and follow-up work. The Wiki is a revision-pinned retrieval mirror. Raw
samples belong in retained assurance evidence or immutable release assets, not
inside this narrative package.
