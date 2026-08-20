# Findings

## NIE-F01: Idle must be positive evidence, not absence of visible motion

Apple's guidance rejects polling, unnecessary timers, and updates to invisible content. Alpine therefore treats zero submitted frames as necessary but insufficient. Qualification also advances the main run loop, checks callback and presentation counters, confirms frame slots are empty, and observes retained-memory behavior.

Evidence: NIE-S01, NIE-S03, NIE-S07. Strength: E3 architectural and performance-design evidence.

## NIE-F02: Alpine-owned counters and operating-system counters answer different questions

`SurfaceSnapshot` can prove whether Alpine admitted, submitted, or directly presented work. `TASK_POWER_INFO` and `powermetrics` can observe wakeups and system power behavior, but cannot identify Alpine frame semantics by themselves. A valid bundle retains both instead of deriving one from the other.

Evidence: NIE-S04, NIE-S05, NIE-S07. Strength: E3.

## NIE-F03: Platform-idle wakeups are a high-value subset

The public task information contract separates total interrupt wakeups from platform-idle wakeups. Chromium's pinned implementation describes the latter as the subset that causes the package to leave idle state. Alpine records both and does not collapse them into a fabricated aggregate score.

Evidence: NIE-S05, NIE-S06. Strength: E2 adoption evidence for metric selection.

## NIE-F04: Energy Impact is not a portable benchmark metric

The local `powermetrics` contract describes estimated power and process energy as optimization signals with platform-specific interpretation. Alpine may use them for paired same-machine qualification after A/A calibration, but not for cross-device or universal framework claims.

Evidence: NIE-S02, NIE-S04. Strength: E3 claim-boundary evidence.

## NIE-F05: An invalidation control prevents a vacuous zero

A broken display link, detached layer, or non-running callback path can produce zero submissions. Every hosted idle experiment therefore includes a control that requests a new scene revision, observes exactly one additional submission and direct-present call, and then returns to quiescence. Physical evidence separately observes compositor presentation.

Evidence: NIE-S07 and independent experimental-design reasoning. Strength: E3.

## NIE-F06: Hosted and physical evidence are complementary

Hosted Apple Silicon CI is appropriate for deterministic scheduler regression gates. It cannot guarantee physical screen topology, compositor occlusion, power-source stability, or thermal control. Physical closure requires a pinned machine and retained raw samples.

Evidence: NIE-S02, NIE-S04, NIE-S07. Strength: E3.
