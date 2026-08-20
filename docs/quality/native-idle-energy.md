# Native idle wakeup and energy protocol

This protocol is the physical acceptance contract for Task #237. The companion [research package](../research/native-idle-energy/index.md) records source provenance, findings, experiments, and decisions.

## Required environment identity

Record and hash:

- Full Alpine commit SHA and dirty-state proof.
- Mac model identifier, CPU, GPU, memory, and serial-redacted hardware profile.
- macOS product version, build, kernel, SDK path, and Xcode or Command Line Tools version.
- Display identity, mode, refresh rate, scale, HDR state, connection, and topology.
- AC or battery source, battery percentage, Low Power Mode, thermal state, and ambient assumptions.
- Tool paths and versions for `powermetrics`, `xctrace`, the compiler, and the evidence verifier.
- Workload schema, font identity, scene revision, settlement duration, sample interval, sample count, and randomization seed.

## Required states

1. Clean visible and unobscured.
2. Hidden through the production AppKit window path.
3. Minimized through the production AppKit window path.
4. Physically occluded by a real opaque peer window.
5. Explicit one-frame invalidation control after each idle family.
6. Validation-only continuous-redraw negative control for instrument sensitivity.

## Measurements

- Alpine callback, request, submission, presentation, frame-slot, allocation, upload, and retained-resource snapshots.
- Per-process total interrupt wakeups and platform-idle wakeups from public task information.
- Machine-readable `powermetrics` task, CPU, GPU, package-idle, and estimated-power samples where supported.
- RSS and physical-footprint series, start and end ownership snapshots, and post-close drain.
- Thermal, display, power-source, and process-liveness observations throughout the window.

## Procedure

1. Build the exact clean revision in the accepted release profile.
2. Capture environment identity before the first run and after the last run.
3. Calibrate control/control variance in randomized paired windows.
4. Predeclare settlement, sample interval, sample count, and invalidation bounds from calibration.
5. Randomize state order within each independent window.
6. Advance the main run loop during every state.
7. Run the hosted invalidation control and require exactly one new submission and direct-present call. In `hosted-direct` mode, inject one validation-only post-commit observation after real submission to drain frame ownership; never count it as compositor evidence. Require compositor presentation separately in physical evidence.
8. Run the negative control and require wakeup and energy sensitivity above the calibrated floor.
9. Hash raw artifacts before analysis and retain analysis output separately.
10. Repeat across at least four independent windows and twenty paired runs, increasing samples when calibration requires it.

## Absolute acceptance invariants

- Zero Alpine Metal submissions after settlement in every idle state.
- No callback or presentation drift after settlement in the hosted structural test.
- Empty occupied and submitted frame slots at every terminal observation.
- No live Alpine native owners after close and drain.
- Exactly one additional real submission and direct-present call for each hosted invalidation control. A hosted synthetic terminal observation proves only bounded ownership drain; the physical control must independently observe one compositor presentation.
- No missing, malformed, or hash-mismatched evidence artifact.

## Calibrated acceptance bounds

- Total interrupt wakeup and platform-idle wakeup rates remain within the predeclared clean-idle bound.
- Same-machine estimated energy does not exceed the calibrated non-inferiority margin.
- RSS and physical footprint show no positive steady-state slope beyond the calibrated detection floor.
- The negative control separates from clean idle by the predeclared effect-size confidence interval.

## Invalidation

Reject a run for correctness failure, nonzero idle submission, failed control, thermal drift, display drift, power-source change, workload mismatch, process interference beyond the declared limit, missing raw evidence, hash mismatch, positive retained-memory slope, or statistical inconclusiveness.

## Claim language

An accepted result may state that the named Alpine revision met the declared native idle submission, wakeup, energy, and residency bounds on the named fixed-hardware environment. It may not state that Alpine is universally zero-energy, faster than GPUI, or more efficient than another editor without a separate matched comparator protocol.
