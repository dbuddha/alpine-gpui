# Mechanism evidence ledgers

An evidence ledger is a claim control surface, not a marketing scorecard.

## Row schema

Each material row records:

- Stable mechanism ID and subsystem.
- Origin classification and exact source identity.
- Alpine source location and implementation revision.
- Behavior retained, modified, strengthened, rejected, or superseded.
- Correctness evidence and remaining uncertainty.
- CPU, GPU, latency, allocation, and residency evidence when relevant.
- Workload and environment identities.
- Implementing issue, pull request, CI run, and retained artifact.
- Evidence level and claim state.
- Missing experiment, review trigger, and supersession link.

## Evidence dimensions

Keep deterministic invariants, avoided work, measured local improvements, and
comparative results distinct. A bounded cache is not total-memory evidence. A
warm zero-rasterization counter is not a GPUI speed comparison. A control quad is
not a realistic code viewport.

## Promotion

Use E0 pointer, E1 pinned primary, E2 triangulated, E3 reproduced, and E4
qualified. Architecture adoption normally needs E2. Measured performance design
needs E3. Comparative dominance needs E4.

Use claim states such as `unclaimed`, `hypothesis`, `implemented`, `reproduced`,
`qualified`, `invalidated`, and `superseded`. Never promote by rewriting prose;
append the evidence event and preserve the previous state.
