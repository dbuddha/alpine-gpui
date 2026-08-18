# WGPU experiment protocol for Alpine

These experiments are candidates, not approved shipping dependencies. A lab
implementation requires a narrow accepted task and dependency record. Results
must satisfy the [comparator protocol](../../quality/comparator-protocol.md).

## Experiment admission

Before any run:

1. Pin Alpine, WGPU, Rust, macOS, hardware, display, font, shader, and workload
   identities.
2. Build WGPU only in an isolated lab target that cannot enter Alpine Studio's
   feature or dependency graph.
3. Prove both adapters consume the same canonical scene trace and produce the
   same semantic omission log.
4. Validate output against Alpine's independent CPU oracle before comparing the
   adapters to each other.
5. Record adaptation separately. Neither adapter may receive a preprocessed
   representation that hides work attributed to the other.
6. Reject performance collection until correctness, lifecycle, and residency
   admission are green.

## WGPU-X001: offscreen differential correctness

**Question.** Can safe WGPU serve as an independent GPU differential oracle for
Alpine scene semantics?

**Workloads.** Solid quad, clipped quad grid, monochrome glyph grid, realistic
code viewport, selection and caret overlay, small scroll delta, and resize.

**Controls.** Use one canonical linear-color scene, identical physical extent,
identical painter order, identical clipping, identical atlas bytes, and explicit
sRGB encode policy. Shader source and compilation paths remain independent.

**Evidence.** CPU oracle output, Alpine Metal readback, WGPU Metal readback,
tolerant pixel diff, omission log, validation output, and exact adapter identity.

**Acceptance.** No semantic mismatch. Pixel tolerances must be justified by
format and raster behavior, not selected after seeing results. Agreement is
differential evidence, not proof that either GPU path is independently correct.

## WGPU-X002: lifecycle and surface outcome matrix

**Question.** Do Alpine and WGPU expose or recover from the same externally
observable window lifecycle without hangs, unbounded work, or false success?

**Cases.** Hide, show, minimize, restore, zero-size resize, live resize,
occlusion, screen move, sleep, wake, close before commit, close after commit,
device interruption, missing drawable, and acquisition timeout.

**Controls.** This is a product-adjacent behavior comparison, not a renderer
timing comparison. Each implementation may use its native event integration,
but the final visible and lifecycle state must match.

**Evidence.** Acquires, submissions, completions, presented frames, recovery
classification, current and peak in-flight resources, timeout, and post-close
drain.

**Acceptance.** Zero idle submissions, no hang, no stale success, bounded
resources, and deterministic close. Different internal recovery is permitted
when external behavior and evidence are equivalent.

## WGPU-X003: prepared-scene stage profile

**Question.** For matched prepared scenes, what costs occur in adaptation,
upload, encode and commit, GPU completion, and presentation?

**Stages.** Trace decode, normalization, adapter scene creation, upload
preparation, command encoding, commit, GPU completion, presentation timestamp,
and end-to-end input-to-present where available.

**Protocol.** Calibrate A/A variance, randomize paired AB and BA order, separate
cold and warm runs, use at least the comparator protocol's required independent
hardware windows and paired samples, retain invalid runs, and report p50, p95,
p99, effect size, and confidence intervals.

**Claim boundary.** A favorable result supports only the named workload, stage,
hardware, and revisions. It does not prove that Alpine is universally faster
than WGPU or that either is the fastest UI framework.

## WGPU-X004: upload churn and retained capacity

**Question.** How do Alpine's fixed frame slots and a tuned WGPU staging strategy
behave under small updates, bursts, cancellation, and pressure?

**Cases.** Steady caret updates, small scroll deltas, full viewport replacement,
large paste redraw, alternating small and large uploads, canceled frame,
unsubmitted prepared frame, allocation failure, and close with work in flight.

**Evidence.** Allocation count, bytes allocated, bytes copied, current and peak
capacity, reused bytes, shed bytes, in-flight count, steady-state slope, process
footprint, private dirty memory, and post-close delta.

**Acceptance.** Both paths must have explicit bounds and terminal release.
Unbounded staging growth is a correctness failure and invalidates timing.

## WGPU-X005: validation fault corpus

**Question.** Which malformed Alpine scenes or lifecycle actions are rejected by
the CPU/model boundary, Alpine Metal boundary, and WGPU adapter boundary?

**Corpus.** Non-finite geometry, overflowing counts, invalid clips, atlas bounds,
unsupported format, zero extent, wrong device, stale revision, stale resize
epoch, duplicate completion, invalid transition, device loss, and resource use
after logical close.

**Acceptance.** Alpine must reject every state its public API forbids before
native undefined behavior. WGPU rejection can identify a missing Alpine check,
but WGPU acceptance cannot widen Alpine's contract.

## WGPU-X006: dependency and binary boundary

**Question.** What would a lab-only WGPU adapter cost before any shipping
decision?

**Evidence.** Cargo feature graph, transitive crates, build time, incremental
build time, binary sections, startup time, initialized threads, loaded dynamic
libraries, process footprint before first frame, unsafe-code inventory, and
license report.

**Acceptance.** The experiment target remains unreachable from shipping Studio
features. A future dependency AEP must use measured evidence from this experiment
rather than ecosystem popularity.

## Invalid experiment conditions

- Different visible output, text, clipping, selection, or final document state.
- WGPU performs shader compilation during a stage where Alpine uses a compiled
  library unless compilation is disclosed as adaptation.
- One path omits validation or lifecycle work required of the other.
- Thermal, power, display, font, OS, adapter, or revision identity drifts.
- Process memory is reported without per-owner accounting, or owner accounting
  is reported without process footprint.
- Failed, timed-out, or semantically invalid runs are silently removed.
- The experiment target leaks into the shipping dependency graph.

## Implementation order

1. Stabilize Alpine code-viewport and glyph trace semantics.
2. Approve an isolated WGPU lab dependency and lock exact features.
3. Implement WGPU-X001 and WGPU-X005 first.
4. Add lifecycle WGPU-X002 only after the lab has a real native surface.
5. Calibrate before running WGPU-X003 or WGPU-X004.
6. Record results as immutable artifacts and update the case study only after
   the evidence is accepted.
