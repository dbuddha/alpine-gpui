# CI decision evaluator rubric

Evaluator-only input. Do not install this file with the skill or expose it,
expected answers, trial labels, or earlier conclusions to trial agents.

Use the established engineering-skill evaluation protocol. Before trials, freeze
baseline and candidate skill revisions, raw-input corpus, model/harness/tools,
permissions, actual instruction-loading trace, scenario applicability, scoring,
randomized paired ordering, and uncertainty treatment. Use three isolated trials
per treatment and scenario as the exploratory minimum. Additional trials or an
inconclusive result are required when the uncertainty rule does not support a
claim. A reviewer reading the skill is not a trial executor or independent grader.

## Scoring

For every scenario score evidence fidelity, corrective specificity, and execution
economy from 0 to 2: 0 is wrong or unsupported, 1 identifies the issue without a
complete actionable control, and 2 provides a source-bound, discriminating,
bounded action. Score uncertainty/authority handling separately on the same scale.
Record omissions, unnecessary blocking, tool calls, tokens, elapsed time, and
human interventions. Do not award evidence fidelity for merely naming tools.

| Scenario | Mandatory distinction | Critical failure |
| --- | --- | --- |
| ci-selection | Whole source/base diff, both rename endpoints, reverse/non-Cargo consumers, conservative unknowns, discriminating selection controls | Admits a partial fallback diff or silently omits consequential inputs |
| ci-baseline | Baseline package/features/cfg/environment and mutation-copy correspondence; unmutated failure is not a caught mutant | Skips a failing baseline without qualified replacement or credits false mutant kills |
| ci-admission | Concurrency versus deduplication, source/tested-base/risk identity, fresh metadata, privileged/untrusted boundary | Reuses incompatible or canceled evidence, skips fresh metadata, or expands privilege |
| ci-sharding | Exact inventory union/disjointness, unviable versus caught, missing outcomes, measured cost, timeout/artifact headroom | Reports unviable/missing outcomes as successful assurance or drops obligations |
| ci-timing | Queue, critical path, summed cost, cold/warm states, equivalent work, calibrated bounds, verification of Zed claims | Claims the stated speedup or universal deadline from incomparable observations |
| ci-shell | Each script needs its own syntax invocation; command fixtures and canonical/hosted acceptance are distinct; source remains pinned | Claims every script was parsed or that the unpublished candidate is accepted |

Prespecify a targeted improvement and reject any candidate critical failure,
including one shared by baseline. The initial exploratory decision requires an
increase of at least one point in median corrective-specificity score for a
prespecified weak scenario, no decrease in other median criterion scores, and no
more than 25 percent median tool-call or token overhead across paired trials.
Report a one-sided paired randomization test for the targeted criterion and
retain all outcomes; p greater than 0.05 is inconclusive, not accepted improvement.
With only three pairs this will often require more data. Do not add observations
opportunistically without a prespecified extension/stopping rule. Never promote
the skill's effectiveness from these unexecuted definitions.
