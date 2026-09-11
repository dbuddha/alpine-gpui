# CI evidence and source contract

## Authority

Load the scoped repository policy and the approved issue before acting. Use
`docs/project/ci-intervention-plan.md` for the stable intervention contract,
GitHub for live status, and the existing evidence registry for accepted claims.
`docs/SUMMARY.md` is navigation. The Wiki is a generated, audited projection.
None of these skills relaxes source/base identity, authorization, or merge gates.

## Minimum records

Selection records bind source/head, tested base, merge base, paths, risk labels,
graph/policy revision, selected obligations, reasons, fallback, and omissions.
Execution records additionally bind toolchain/target/features/cfg, runner/cache
state, discovered/selected/executed/ignored/terminal counts, stage durations,
artifact digests, and explicit passed/failed/blocked/inconclusive/not-required
disposition. Record resolution and instrumentation overhead for timing claims.

Equivalent inventories are necessary but not sufficient: changed package
features, process isolation, environment, shared resources, or test ordering can
change behavior. Review actual commands and production correspondence. Retain
unfavorable samples and unviable, interrupted, or missing mutant outcomes.

## Primary source retrieval

- [Cargo metadata](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html):
  pin the Cargo version and feature/target resolution used to derive package edges.
- [cargo-mutants baselines](https://mutants.rs/baseline.html): baseline execution
  establishes that unmutated tests pass and informs timeout selection. Separately
  running tests can support an accepted reuse design, not an automatic waiver.
- [cargo-mutants sharding](https://mutants.rs/shards.html): pin the installed tool,
  inventory, order, filters, sharding method, and number of partitions.
- [GitHub concurrency](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-workflow-concurrency):
  concurrency control is not completed-work deduplication or acceptance authority.
- [Nextest process isolation](https://nexte.st/docs/design/why-process-per-test/):
  test-runner changes need resource and cleanup equivalence, not just a faster run.
- [Pinned Zed workflow source](https://github.com/zed-industries/zed/blob/a57ba9b17c433ea1ebfdec8f649f4fa5a402d03b/tooling/xtask/src/tasks/workflows/run_tests.rs):
  use as workflow research, not a replacement renderer comparator or proof of
  Alpine selection completeness. Refresh a research pin only through its normal
  review; keep source observation and inferred intent distinct.

Use the runtime measurements and actual pinned source, not reputation or a skill
title, to justify changes. An aggregate wall-clock improvement with a different
inventory, runner, cache, or failure outcome is not a demonstrated speedup.
