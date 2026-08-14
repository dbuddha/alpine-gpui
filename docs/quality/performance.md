# Performance qualification

A performance or memory requirement must define the user-visible metric,
deterministic workload, baseline revision, machine identity, OS and developer
tools, GPU and display state, power and thermal conditions, warmup, samples,
distribution comparison, regression threshold, and correctness constraints.

Hosted timing is diagnostic only. Blocking gates run on qualified fixed
hardware. TLA+ and Kani may establish structural properties such as bounded
queue depth, no idle submission, or logical capacity, but they cannot establish
elapsed time, GPU throughput, energy, allocation count, or resident memory.

Zed-relative results use `alpine-scene-trace/v1`, `alpine-journey/v1`, and
`alpine-qualification/v1`. Renderer-only, full-Zed-path, and product-journey
comparisons are reported separately. The assurance tool rejects measurement
before required equivalence and environment qualification. A qualified state
records raw artifacts and at least three independent hardware windows; fixture
data validates this protocol but is not production benchmark evidence.

An optimization is acceptable only if its correctness, accessibility, and
memory constraints still pass. Benchmark evidence is revision-scoped and is
attached to release qualification rather than copied into standing prose.
