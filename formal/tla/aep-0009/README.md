# Assurance lifecycle model

TLC 1.7.4 checks AEP-0009-C01, C02, C03, and C05 over a finite monotonic
workflow. `Requirements` are approved GitHub Requirements. `EvidenceKinds` are
qualified evidence classes. `approved`, `implemented`, `recordedEvidence`, and
`closed` map to GitHub labels, pull requests, registry records plus checks, and
hierarchy closure. `capabilityClosed` maps to completed Capability state.

`QualifiedClosure` prevents closure before approval, implementation, and all
required evidence. `CapabilityClosure` prevents premature Capability closure.
`CanComplete` checks progress under weak fairness. Pull-request bounds are two
Requirements by two kinds; nightly uses three by three. The model excludes API
failures, review quality, mutable GitHub state during a run, evidence semantics,
and implementation bugs in the validator or workflows.

`Faulty.cfg` sets `FaultyClosure` and must demonstrate that TLC finds premature
Requirement closure. Rust conformance lives in the validator tests, policy
fixtures, and hierarchy fixtures. The model is not a refinement proof.
