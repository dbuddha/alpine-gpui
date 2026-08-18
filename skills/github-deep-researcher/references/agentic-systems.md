# Agentic-systems research

## Evaluation identity

| Dimension | Required identity |
| --- | --- |
| Model | Provider, model ID, dated revision, reasoning mode, sampling |
| Harness | Repository, commit, orchestration and context policy |
| Prompt | System, developer, task, templates, evaluator instructions |
| Tools | Names, versions, schemas, permissions, sandbox, network, scopes |
| Task | Issue text, base commit, patch boundary, test contract |
| Environment | OS, architecture, dependencies, caches, limits |
| Trials | Count, seeds or nondeterminism policy, retries, stopping rule |
| Trajectory | Messages, calls, outputs, errors, approvals, compaction, handoffs |
| Grader | Tests, judge and prompt, rubric, flaky-test handling, adjudication |
| Outcomes | Pass, partial, failure class, cost, time, tokens, intervention |
| Contamination | Public exposure, overlap risk, leaked tests, prior state |

## Fair comparison

Hold task, base revision, environment, permissions, budgets, and grader constant. If scaffold differs, the result compares systems, not models. Randomize order when drift matters. Retain failed trajectories.

## Failure taxonomy

Use requirement misunderstanding, missing context, endless exploration, wrong architecture, incorrect or incomplete patch, test gaming, tool misuse, permission block, environment failure, context loss, verification omission, fabricated evidence, and cost or time exhaustion. Allow contributing factors and one primary stop reason.

## GitHub fit

Issue owns task contract; branch and PR own proposed solution; checks own deterministic grading; Project owns cohort and state; research package owns method, trajectories, findings, and limitations; release assets retain immutable large bundles.

SWE-bench established repository issue resolution against tests. Long-horizon evaluation also requires resolvable human-verified tasks, contamination controls, unified scaffolds for model comparisons, and trajectory failure analysis: https://arxiv.org/abs/2509.16941

Do not report one lucky run, compare different permissions as model quality, hide retries, discard failures, use mutable model aliases without date, accept judge-only grading when deterministic tests exist, mix scaffold and model effects, expose hidden tests to one system, or treat benchmark success as production autonomy.
