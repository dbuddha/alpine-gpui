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

The implemented `alpine-aa-calibration/v1` boundary validates raw A/A evidence
before any performance threshold can be proposed. One record requires identical
base and candidate renderer revisions, an exact workload, at least twenty runs
across four distinct windows, unique randomization identities, both execution
orders with at most one-pair imbalance per run, qualified offline-shader
environments, an explicit measurement stage and clock, separate cold or warm
samples with a declared warmup count, ordered window times, and a recomputed
hash of strict LF-normalized raw paired samples. Its report
uses deterministic integer parts-per-million deltas only to expose observed
drift and order effects. Fixture reports say `fixture-only`; physical records
may say `protocol-ready`. Neither status is a confidence interval, equivalence
margin, sample-size approval, blocking gate, or performance claim. Those require
real physical-hardware data and a later owner-approved statistical decision.
