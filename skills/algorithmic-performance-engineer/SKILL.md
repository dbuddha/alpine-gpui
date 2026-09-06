---
name: algorithmic-performance-engineer
description: Select and validate editor algorithms and data layouts using workload models, complexity bounds, allocation, locality, and measured end-to-end bottlenecks.
---

# Algorithmic performance engineering

Start with the actual operation distribution and correctness contract. Define
input size, visible working set, update locality, query frequency, concurrency,
retention lifetime and adversarial cases. Count time and bytes, not just Big O.

## Make the optimization decision explicit

For a proposed change, compare a simple baseline and credible alternatives on
worst-case/amortized/expected complexity, constants, cache locality, copies,
allocation, synchronization, cancellation, maintainability and failure bounds.
An expected hash lookup is not a deterministic worst-case constant-time guarantee.
Parallelism adds scheduling and ownership cost; do not add workers to a tiny
foreground operation or a presentation-limited workload.

Use Amdahl's bound `1 / ((1 - p) + p / s)` to test whether accelerating fraction
`p` by `s` can materially change the measured endpoint. For GPU work, use arithmetic
intensity and measured bandwidth/throughput as a roofline-style diagnostic, not
marketing peak rates. Include queueing and synchronization outside the kernel.
Define whether a byte count is logical payload, capacity, copied bytes or physical
residency before comparing memory.

Read the [editor decision table](references/editor-algorithms.md) when choosing a
data structure. Keep an independent oracle and adversarial corpus. A faster
algorithm with lost Unicode, stale-result or durability behavior is not valid.

## Production-oriented experiments

- Change one measured dominant cost. Compare distributions and allocations on
  realistic cold/warm sizes, long lines, Unicode, large edits and memory pressure.
- Prove cache hits avoid upstream work: lookup before materializing text, shaping,
  rasterizing or uploading. Bound keys, values, scratch, temporary peaks and
  eviction metadata; account for snapshot/undo retention as well as live text.
- Separate throughput from tail latency. Incremental work must have invalidation
  and cancellation proofs, not just fast steady-state benchmarks. A small edit
  may require downstream lexical invalidation across many lines.
- Use differential/property tests for mutation, indexing and transformations;
  bounds/proofs for arithmetic and state; process/physical tests for scheduling
  and residency. Formal tools do not prove elapsed time or all allocator behavior.
- Approximation requires an accepted error contract and a checked fallback.
  Never approximate text contents, save durability, ordering or accessibility to
  gain speed. Reduced shader precision needs output-equivalence evidence.

Return a cost model, alternatives, crossover conditions, evidence-backed choice,
failure bounds, tests and measured tradeoffs. Record the algorithm's primary
reference and Alpine transformation in the lineage log. Read actual papers or
official specifications before asserting niche results; use
`$github-deep-researcher` when contradiction or source quality is decisive.
