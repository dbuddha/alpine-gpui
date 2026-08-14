# Portable value admission model

TLC 1.7.4 checks the finite admission design behind AEP-0016-C01 and C02.
`candidate` represents one scalar supplied to a size or color constructor.
`Choose`, `Accept`, and `Reject` map to input selection and the `Option` result.
`AcceptedIsValid` prevents invalid admission, `AllIntersectionsSafe` checks
finite interval containment independently for each rectangle axis, and
`CanAccept` verifies progress through at least one valid value under weak
fairness.

Pull-request bounds use raw model values `0..3` and valid values `1..2`;
nightly uses `0..6` and `2..4`. Values outside the valid subrange represent
invalid negative, non-finite, or out-of-range inputs. This abstraction excludes IEEE-754 details,
multi-channel correlation, floating-point rectangle arithmetic, allocation,
concurrency, native conversion, and elapsed time. Rust constructor tests are the conformance
evidence. Kani separately checks compiled Rust across its stated symbolic
domains. `Faulty.cfg` relaxes admission and must produce a counterexample.
