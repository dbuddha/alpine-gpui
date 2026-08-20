# Experiments

## NIE-E01: Hosted state-transition discrimination

Purpose: reject regressions that continuously callback, submit, present, or retain frame slots after settlement.

States: clean visible, hidden with `orderOut`, minimized with `miniaturize`, and restored visible.

Control: request one strictly newer scene revision after restore. Require exactly one additional real submission and direct-present call, terminal frame ownership, and a return to paused display-link state. Hosted runners use an explicit validation-only post-commit observation to drain the real submitted frame when Core Animation supplies no presented callback. That injected observation is not compositor evidence.

Evidence: the native `native_idle` executable running under `alpine_native_validation` on hosted Apple Silicon macOS.

Claim ceiling: scheduler, direct-present invocation, and ownership behavior on the hosted environment only. Physical compositor presentation, wakeup, and energy remain unqualified.

## NIE-E02: Fixed-hardware A/A calibration

Purpose: determine natural same-machine variance before setting a blocking wakeup or power threshold.

Method: run identical control/control pairs in randomized order across independent windows. Retain raw task wakeups, `powermetrics` plist samples, RSS, Alpine snapshots, thermal state, power source, and environment identity. Report p50, p95, p99, paired effect size, and confidence intervals.

Activation rule: no threshold becomes blocking until calibration demonstrates a stable detection floor across at least four independent windows.

## NIE-E03: Physical idle-state qualification

Purpose: qualify visible, hidden, minimized, and physically occluded idle behavior on one pinned Apple Silicon Mac.

Method: execute each state for a predeclared settlement and sampling duration. Require zero Alpine submissions after settlement, bounded callback and wakeup rates, no positive retained-memory slope, and a passing invalidation control.

Invalidation: reject thermal drift, display drift, workload mismatch, missing raw artifacts, failed control, nonzero idle submission, or statistically inconclusive evidence.

## NIE-E04: Deliberate continuous-redraw negative control

Purpose: prove the operating-system sampling path detects meaningful extra work.

Method: a validation-only workload admits frames at a fixed bounded cadence without changing scene semantics. It is never a shipping mode. The observed wakeup and power distributions must separate from clean idle before physical results are accepted.

Claim ceiling: instrument sensitivity, not product performance.
