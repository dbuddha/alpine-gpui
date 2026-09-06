# Engineering skill evaluation and guarded evolution

Requirement #564 and Experiment #566 own behavioral acceptance. Structural gates
prove packaging and installation only. The canonical scenario inventory is
`assurance/agent-skills/v1/scenarios.tsv`; prompts and evaluator rubric are separate
files beside it. An unexecuted suite is not a passed suite.

## Forward evaluation

1. Freeze the treatment: initial rollout compares the pinned three GitHub skills
   with the seven manifest skills; this estimates package, not individual-skill,
   effects. Evolution compares the accepted skill version with one candidate.
   A no-skill ablation is optional only where the harness genuinely excludes skill
   exposure without changing repository policy or other task context.
2. Keep task, product source, permitted raw inputs, model/provider, harness, tools,
   permissions and environment identical across treatments. Pin skill revisions
   separately. Isolate sessions, installed-link resolution, retrieval and cached
   context; separate output directories alone are insufficient. Retain actual
   loaded-instruction content/hashes, resolved paths/revisions and load order,
   including repository instructions and indirect loads. Fail closed if treatment
   contamination cannot be excluded. Withhold rubric, expected answers and prior
   conclusions from trial agents; permit no live mutations.
3. Before trials, freeze scenario-specific criterion applicability, scoring
   anchors, meaningful improvement thresholds, aggregation, uncertainty treatment
   and stopping rules. Include the rubric's paired controls and held-out unlabelled
   routing cases; score loading separately from downstream decision quality.
4. Run at least three baseline and three candidate trials per affected scenario
   with prespecified paired or randomized ordering. Three trials are an exploratory
   minimum, not blanket effectiveness proof; claims require adequate evidence for
   the prespecified uncertainty rule, otherwise report inconclusive.
   Keep identities, complete outputs and output hashes, including refusals,
   omissions, failures and interruptions. Do not count a critic's code review as
   these trials or self-grade the implementation as independent evidence. Record
   tool calls, tokens, elapsed time, human interventions and unnecessary blocking.
5. An independent evaluator, blinded to treatment where feasible, records critical
   failures, criterion scores, identity and disposition. Any candidate critical
   failure rejects the candidate, even if the baseline shares it. Accept only
   prespecified meaningful improvement with no critical failure; account for
   overhead and unnecessary blocking. Inconclusive comparisons preserve baseline.
6. Retain a manifest binding every input and result. Verification must reject
   tampered/missing prompts, outputs, rubrics, revisions and hashes. Empty trial
   counts, missing executors or unavailable evaluation tools are not success.
7. Run packaging/install, documentation, retention and full required CI gates.
   Bind exact PR head, tested base, hosted run, merge revision and post-merge run.
   Append an accepted change only after exact-main evidence and evaluation pass.

The trial manifest must include treatment definitions; skill, baseline, candidate,
repository, model, harness and scenario revisions; prompt/tool/permission/environment/
rubric hashes; actual loaded-instruction traces and isolation evidence; trial order
and number; output path/hash; prespecified scoring and uncertainty rules; routing
and criterion scores; critical failures; tool/token/time/intervention/blocking
measures; evaluator and blinding; disposition; PR source head, tested base, hosted
run, merge revision and post-merge run. A hash proves integrity of retained bytes,
not truth of the grader's conclusion. This protocol never supersedes repository
policy or required gates.

## Bounded improvement loop

Start from a demonstrated failure or user correction. Make one targeted candidate;
do not accumulate generic rules. Stop after three unsuccessful candidate revisions
and preserve the prior accepted skill and unfavorable evidence. Standing autonomy
does not permit scope expansion, security bypass, arbitrary model/tool permission
changes, weakened CI, or unsupported claims.

`assurance/agent-skills/v1/evolution.tsv` is append-only accepted history. Its
header alone means no accepted skill-evolution result has been recorded. Keep live
implementation/evaluation status in Issues and Projects, not this protocol or the
Wiki. The initial scenario package and structural checker do not implement the
independent evaluation runner or its tamper verifier; those remain #566 work.
Accepted baseline/candidate effectiveness evidence also remains incomplete; these
protocol changes add no evaluation results or E3 claims.
