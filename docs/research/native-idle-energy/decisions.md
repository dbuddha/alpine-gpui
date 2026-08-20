# Decisions

## NIE-D01: Use two authorities

Alpine-owned counters are authoritative for frame admission, submission, direct-present calls, presented-time callbacks when available, and resource ownership. Public operating-system counters are authoritative for observed process wakeups and system power samples. Neither substitutes for the other.

## NIE-D02: Keep hosted qualification blocking and physically scoped

Hosted structural qualification is a blocking regression gate because it is deterministic and available on every relevant pull request. Physical energy qualification is a release and milestone gate because its environment cannot be guaranteed by hosted CI.

## NIE-D03: Reject private power APIs

Alpine will not ship or qualify through private `pm_sample_task`, `pm_energy_impact`, or equivalent APIs. Public `TASK_POWER_INFO`, documented tools, and first-party counters are sufficient for the scoped claim.

## NIE-D04: Separate absolute invariants from calibrated bounds

Zero post-settlement Alpine submissions, empty terminal frame slots, and a passing invalidation control are absolute invariants. Wakeup and energy thresholds are calibrated bounds derived from A/A variance on the pinned machine.

## NIE-D05: Preserve raw evidence and narrow claims

Every accepted run retains raw samples, environment identity, workload identity, tool versions, hashes, and invalidation decisions. Results support only the named machine, state, revision, workload, and metric. They do not authorize a cross-device or universal performance claim.
