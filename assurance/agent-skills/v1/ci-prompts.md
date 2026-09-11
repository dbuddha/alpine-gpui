# CI decision scenarios

These are unexecuted scenario definitions. Evaluation uses the established
engineering-skill protocol. Supply only the selected task and permitted raw
inputs to trial agents; do not expose the evaluator rubric or prior conclusions.
Scenario quantities are fixtures, not accepted measurements of current main.

## ci-selection

A proposed classifier selects a crate's tests when that crate's Rust files change.
A pull request changes a runtime crate in its first commit and only documentation
in its second commit. It also renames a shader into an asset directory and adds
a JSON fixture for a native process test. The classifier falls back to HEAD^ when
the requested base is missing. Describe the needed selection behavior, evidence,
and a bounded correction without introducing another build system.

## ci-baseline

A native mutation job reports an unmutated failure. Its baseline tested package A;
the mutation command tests packages A and B under a validation cfg. A developer
proposes skipping the baseline because package A passed in an ordinary checkout.
Define the next investigation, safe admission conditions, and what the existing
results do or do not prove.

## ci-admission

Four label edits on one PR head admitted four expensive suites. The workflow uses
concurrency cancellation, and one completed run exists for an older tested base.
One new label changes the required assurance families. Propose a bounded design
that avoids duplicate work while preserving metadata, authorization, and merge
correctness. State which evidence may be reused and why.

## ci-sharding

A 64-mutant shard exceeded its job deadline. Its retained artifact lists 47 caught
and 11 unviable mutants, with no terminal outcome for six entries. The proposed
change doubles shard count, increases job timeout, and reports 58 of 64 successful.
Assess that proposal and specify the measurements and controls needed before
accepting a replacement schedule.

## ci-timing

A warm local command completes in one second. Hosted end-to-end feedback takes
80 minutes, while summed runner execution is six hours. A proposed report says
the pipeline is now 4,800 times faster and all jobs will complete in 15 minutes
because Zed does. Assess the report and propose a credible runtime target and
measurement experiment. Keep correctness requirements unchanged.

## ci-shell

A developer invokes `sh -n first.sh second.sh third.sh`; first.sh is valid, but
second.sh contains a syntax error. Fixture tests of a newly added command pass,
while the canonical policy check fails. The published PR still points at the
previous commit. Explain the source/validation state, the syntax-check mechanism,
and the next safe steps without conflating local edits and hosted acceptance.
