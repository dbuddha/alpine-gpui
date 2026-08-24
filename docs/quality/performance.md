# Performance qualification

A performance or memory requirement must define the user-visible metric,
deterministic workload, baseline revision, machine identity, OS and developer
tools, GPU and display state, power and thermal conditions, warmup, samples,
distribution comparison, regression threshold, and correctness constraints.

Hosted timing is diagnostic only. Blocking gates run on qualified fixed
hardware. TLA+ and Kani may establish structural properties such as bounded
queue depth, no idle submission, or logical capacity, but they cannot establish
elapsed time, GPU throughput, energy, allocation count, or resident memory.

Zed-relative results use versioned `alpine-scene-trace` inputs,
`alpine-journey/v1`, and
`alpine-qualification/v1`. Renderer-only, full-Zed-path, and product-journey
comparisons are reported separately. The assurance tool rejects measurement
before required equivalence and environment qualification. A qualified state
records raw artifacts and at least three independent hardware windows; fixture
data validates this protocol but is not production benchmark evidence.

The retained version 1 renderer-only control is deliberately limited to one
full-viewport clip and painter-ordered solid quads. Version 2 prepared scenes
add bounded axis-aligned clips, one immutable A8 atlas, solid quads, and
monochrome glyphs without placing shaping, rasterization, or adaptation inside
renderer timing. Scroll and resize are identity-bound scene pairs. The decoder
rejects every other primitive or resource until both compared renderers support
it. The assurance CLI validates each trace and emits compact BGRA8 from the independent CPU oracle;
on a supported physical Mac it can emit the same artifact through Alpine Direct
Metal. Scene parsing, adaptation, validation, encoding, completion, and readback
remain distinct measurement stages. A future paired runner cannot place parsing
or adapter work inside one renderer's timed interval and outside the other's.

An optimization is acceptable only if its correctness, accessibility, and
memory constraints still pass. Benchmark evidence is revision-scoped and is
attached to release qualification rather than copied into standing prose.

The first accepted Zed-lab record qualifies no performance. It binds one
solid-quad trace across a retained hosted offline-shader GPUI artifact and a
physical Apple Silicon run of GPUI Metal, Alpine Direct Metal, and the CPU
oracle. Exact readback identity, adapter coverage, and adapter mutation results
are machine-validated under `assurance/lab/`; raw GPL lab artifacts remain in
the lab or GitHub artifact store. Later timing must use the separate calibrated
qualification protocol.

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
