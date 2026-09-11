# Alpine CI job checker

`alpine-ci-job-checker` is the focused CI engineering entry point for the
[approved intervention](../project/ci-intervention-plan.md). It uses the same
manifest, installer, checker, and evaluation protocol as the existing seven
skills. It introduces no model, background worker, shipping dependency, or new
build system. The additional skill has no M1-M5 completion credit.

## Responsibilities and routing

Use it for affected selection, package and non-Cargo dependency edges, native
baseline correspondence, mutation/shard completeness, early failure, compatible
caching, deduplication, and runtime evidence. Product/native implementation,
GitHub live state, canonical documentation/Wiki publication, and comparative
research retain their existing domain owners. Load supporting skills only at
those boundaries, not all skills for every CI change.

The source of installable inventory is `skills/manifest.tsv`. The additional
scenario inventory is `assurance/agent-skills/v1/ci-scenarios.tsv`. This extends
the scenario collection; it does not retroactively alter the initial three-to-
seven-skill treatment or its acceptance under #564/#566. The CI-specific baseline
is the pinned existing seven-skill configuration and the candidate adds only this
skill under otherwise identical evaluation conditions.

## Acceptance and publication

The [engineering-skill evaluation protocol](../quality/engineering-skill-evaluation.md)
remains binding. Packaging, installation, source review, baseline/candidate trials,
independent grading, and merge acceptance are separate evidence levels. The new
scenario and rubric files are definitions, not results. Keep evaluator material
outside the installed skill and withhold it from trial agents.

Use a disposable CODEX_HOME for installer tests. Never replace foreign installed
links or point the user's working skill system at an unaccepted temporary clone.
After an accepted merge, install from the authoritative repository and publish
the Wiki through the existing generated-mirror and drift-audit procedure. A local
skill directory, source branch, and live installed skill are different states.

Track intervention delivery through #510 and preserve skill feedback/evaluation
under #566 or its explicitly accepted successor. Do not reopen completed skill
implementation merely because the CI extension adds a new inventory member.
Append an evolution result only after its actual evaluation and exact-main gates
pass. No improvement result is created by this documentation or scenario set.

## Mutation compiler reuse

PR and main native mutation jobs enable Cargo incremental compilation inside
each fresh mutation copy. Independent ordinary native admission retains mode 0;
each mutation-copy baseline uses mode 1, corresponding to its mutants. Baselines,
selected mutants, fresh test execution, package/features/cfg, debug information,
assertions, timeout policies and terminal receipt requirements remain unchanged.
Automatic per-mutant limits are still calibrated from each actual baseline. Normal
non-mutation CI and scheduled assurance retain their existing compiler defaults.
Native identity v2 records `cargo_incremental`; mode changes invalidate prepared
receipts. Alpine workspace build artifacts do not cross jobs; the separate
pinned-tool cache remains allowed. No test-success result is cached.

The mechanism follows [Cargo's incremental configuration](https://doc.rust-lang.org/cargo/reference/config.html#buildincremental)
and the [cargo-mutants cost model](https://mutants.rs/performance.html), not copied
Zed source. [Task #510](https://github.com/dbuddha/alpine-gpui/issues/510) retains
the measured baseline, candidate protocol and disposition. Local build savings
nominate hosted validation; they do not establish full CI latency, scheduled
assurance completion, native fault recovery or product performance.
