---
name: apple-metal-performance-engineer
description: Diagnose Alpine Metal, AppKit and QuartzCore correctness, presentation latency, GPU work, resource ownership and physical Apple Silicon residency.
---

# Apple Metal performance engineering

Optimize the measured native endpoint, not the label "GPU accelerated". Pin the
device, GPU families, OS, SDK, shader/binary/source identities and workload before
using generation-specific advice. Read the [source and Asahi boundary](references/asahi-metal-boundary.md)
for hardware interpretation; it distinguishes public contracts, reverse-engineered
observations and hypotheses. Do not claim expertise in uninspected future devices.

## Localize before changing

Build a timeline with input receipt, mutation, invalidation, scene build,
adaptation, upload, encode, commit, GPU interval, completion observation, display
callback, target presentation, actual presented time and callback arrival.
Check clock domains, units, omissions and caller boundaries. GPU work overlaps
host waits: neither add them nor label their difference "driver overhead".
Zero or absent actual presentation is missing evidence, not a timestamp to replace
with callback arrival, a target deadline or GPU completion.
Before a new physical experiment, inspect the existing presentation investigation
and require a discriminating hypothesis. A hosted-native pass cannot qualify the
physical Mac, and missing presentation evidence alone does not prove a blank window.

Calibrate ordinary/instrumented A/A and use matched complete endpoints before
selecting an optimization. Preserve unfavorable full submit-readback while adding
narrower endpoints separately. A tiny seven-glyph control is not an editor-scale
viewport regardless of its name. Use representative scale, glyph count, clips,
alpha, selection, scrolling, resize and churn before generalizing a result.

## Resource and scheduling decisions

- Preserve asynchronous completion, bounded frame slots, lifecycle generations,
  stale-callback rejection and deterministic drain. Never recycle upload memory
  while an in-flight GPU operation still owns it.
- Use allocation, upload and atlas-mutation evidence to choose reuse or dirty-row
  updates. Unified memory does not eliminate synchronization, copies, private
  resource costs or transient allocations. Do not sum shared bytes twice.
- Review attachment load/store actions and primitive/vertex traffic when GPU
  counters justify it. Preserve UI painter order and blended glyph semantics;
  game-style opaque sorting, depth buffers or pass splitting are not free wins.
- Distinguish buffer capacity, requested bytes, owned resource accounting, process
  footprint, private dirty memory and unobservable driver allocations. Measure
  cold, warm, churn, recovery, in-process teardown and process exit separately.
- A bounded scheduling tail is an experiment, not a continuous loop. Count
  callbacks and OS-delivered drawables separately from dirty admissions and frame
  submissions. Test cancellation on focus loss, hide, sleep and close. Separate
  unchanged-scene replay from dirty-only wake policies and account for its energy.

## Physical and causal evidence

Use GPU capture, Metal validation and Instruments/System Trace as appropriate;
calibrate observer overhead and keep compilation/competing benchmarks out of
measurement windows. GPU-capable hosted macOS can test Metal equivalence and
diagnostic trends. Physical 60/120 Hz, energy, VoiceOver and target-Mac superiority
need the relevant physical setup. Respect current desktop permission and normal
TCC; never disable security, patch private driver state or relaunch an app merely
to prove it closed. Track the original PID and process-start identity.

Return the supported contract, measured bottleneck, competing hypothesis, bounded
change, correctness/memory tradeoff, raw evidence and claim ceiling. State calibration and statistical limitations alongside measurements.
