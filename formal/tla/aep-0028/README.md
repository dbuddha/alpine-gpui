# AEP 0028 qualification model

`GoldenQualification.tla` models the evidence-ordering contract from an
identified workload through equivalence, environment qualification,
measurement, independent reproduction, and final qualification.

- `passedGates` maps to the manifest equivalence records.
- `workloadMatches` maps to the five equal workload hashes.
- `environmentQualified` maps to the attested environment record.
- `measurementsRecorded` maps to raw measurement artifact records.
- `independentWindows` maps to distinct hardware qualification windows.
- `ValidateEquivalent`, `Measure`, `Reproduce`, and `Qualify` map to pure Rust
  validation in `tools/alpine-assurance/src/qualification.rs`.

The model excludes semantic validity of evidence, TOML parser correctness,
GitHub or runner availability, native GPU behavior, statistical calculations,
and actual model-to-code refinement. Pull requests use four gates and three
windows; nightly adds accessibility and a fourth window. Weak fairness on the
combined transition relation supports the terminal progress property.

`Faulty.cfg` permits measurement directly from `Loaded`. TLC must find a
`MeasurementRequiresEquivalence` counterexample.
