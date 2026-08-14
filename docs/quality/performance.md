# Performance qualification

A performance or memory requirement must define the user-visible metric,
deterministic workload, baseline revision, machine identity, OS and developer
tools, GPU and display state, power and thermal conditions, warmup, samples,
distribution comparison, regression threshold, and correctness constraints.

Hosted timing is diagnostic only. Blocking gates run on qualified fixed
hardware. TLA+ and Kani may establish structural properties such as bounded
queue depth, no idle submission, or logical capacity, but they cannot establish
elapsed time, GPU throughput, energy, allocation count, or resident memory.

An optimization is acceptable only if its correctness, accessibility, and
memory constraints still pass. Benchmark evidence is revision-scoped and is
attached to release qualification rather than copied into standing prose.
