# Performance and memory claim readiness

This page defines when a claim is supportable. It does not contain live benchmark
results or promote claims. The evidence registry, retained artifacts, issues,
pull requests, CI, and release report own revision-specific results.

## Evidence levels

| Level | Meaning | Permitted statement |
| --- | --- | --- |
| E0 | Discoverable pointer | A source or question exists |
| E1 | Pinned primary evidence | The exact source states or implements a bounded fact |
| E2 | Triangulated analysis | Alpine has a source-supported design decision with contradictions and limits |
| E3 | Alpine-controlled reproduction | A mechanism or result was reproduced under a retained environment and workload |
| E4 | Fixed-protocol qualification | A scoped comparative claim passed semantic, hardware, statistical, and retention gates |

Architecture adoption normally requires E2. Performance design decisions need
E3 when they depend on measured behavior. Comparative dominance requires E4.

## Claim classes

| Claim | Minimum evidence | Required admission |
| --- | --- | --- |
| Structural bound, such as three frame slots | Executable invariant plus risk-selected tests | Production contract and failure behavior match the bound |
| Avoided work, such as zero warm glyph rasterization | Deterministic counters and regression | Correct viewport output and cache lifecycle remain green |
| Alpine revision improved a local metric | Reproduced before and after distributions | Same behavior, workload, environment, and build identity |
| Alpine GPUI is faster or more memory-efficient than pinned GPUI for a workload | E4 paired renderer qualification | Semantically equivalent scene, adaptation reported separately, no omitted operations |
| Alpine Studio is faster or more memory-efficient than Zed or Sublime for a journey | E4 paired product qualification | Matched local behavior, normalized and stock configurations reported separately |
| Active editing meets 120 Hz deadlines | Fixed-hardware frame and latency evidence | Named active journey and calibrated display mode; idle remains zero frames |
| Input-to-photon latency | Calibrated optical E4 evidence | Endpoint, actuator or event source, display state, sensor, and raw traces retained |

No editor-only result supports a universal fastest-framework claim. No control
quad supports a realistic text-rendering claim. No cache budget alone supports a
total-memory claim.

## Required identity

Every performance or memory package retains:

- Base, candidate, comparator, adapter, and lab revisions as applicable.
- Workload protocol and hash.
- Environment, OS, hardware, display, power, thermal, build, font, settings,
  language-server, and exclusion identities.
- Semantic, visual, lifecycle, and accessibility admission results.
- Cold and warm classification and randomized paired order.
- Raw samples, invalid runs, p50, p95, p99, confidence interval, and effect size.
- CPU scene-build and main-thread distributions.
- Adaptation, upload, encode, commit, GPU completion, presented-time, and
  event-to-present stages when applicable.
- Physical footprint, private dirty memory, allocator activity, GPU resources,
  cache bytes, peaks, steady-state slope, and post-close delta.
- Issue, pull request, CI run, evidence registry claim, lineage row, and
  historical-log identity.

## Fair-comparison layers

1. Renderer-only compares identical normalized scene semantics after each scene
   is prepared.
2. Adaptation-only reports decode, normalize, allocate, and scene-construction
   cost separately.
3. Framework comparison reports application state to scene, upload, encode,
   completion, and presentation as distinct endpoints.
4. Normalized product comparison uses matched local-editor behavior with
   configurable accounts, AI, collaboration, telemetry, extensions, and plugins
   disabled where possible.
5. Stock-product footprint is reported separately and never substituted for the
   normalized journey.

Sublime Text is externally observed because its implementation is proprietary.
Zed source instrumentation and patches remain isolated in `alpine-zed-lab`.
WGPU remains a research or differential oracle, not a shipping backend.

## Promotion and invalidation

A claim advances only when its confidence interval and accepted
non-inferiority or superiority rule are satisfied across the required
independent hardware windows. Retain inconclusive and unfavorable outcomes.

Invalidate a run for semantic mismatch, missing behavior, accessibility or
lifecycle failure, dropped operations, workload drift, environment drift,
thermal or display drift, missing raw identity, underpowered samples, or
statistical inconclusiveness. Do not repair a failing claim by changing the
workload after results are visible.

## Wording

Use:

> On `<hardware and OS>`, Alpine revision `<sha>` was `<effect and interval>`
> versus `<comparator revision>` for `<workload and metric>` under `<exclusions>`.

Do not use `fastest UI framework`, `always 120 FPS`, `zero memory`, or a
fabricated aggregate performance score.

Current mechanism evidence and missing experiments are retrieved through the
[lineage evidence ledger](../research/alpine-lineage/evidence-ledger.md) and
[experiment queue](../research/alpine-lineage/experiments.md).
