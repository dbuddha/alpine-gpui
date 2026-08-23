# Evidence-aware delivery

Completion is derived from accepted evidence, not issue motion.

## Work semantics

- Capability: end-to-end observable outcome.
- Requirement: approved testable behavior.
- Task: bounded implementation or evidence-producing unit.
- Defect: reproduced failure, correction, and regression.
- Research: reviewed source package, findings, limits, and disposition.
- Experiment: protocol, implementation, raw evidence, analysis, and conclusion.

Each task has one direct parent. Related concerns use links. Dependencies express
required ordering, not preference.

## Closure

A task closes when its named implementation or evidence result is accepted. A
requirement closes only when required child leaves and end-to-end acceptance are
complete. A capability closes only when required approved outcomes are accepted.

Never infer closure from a merged branch, parent counter, Project `Done` field,
documentation statement, or passing narrow test. Detect merged pull requests
whose tasks remain ambiguously open and closed tasks whose evidence is missing.

## Evidence identity

Record exact revision, issue, pull request, CI run, artifact, workload,
environment, exclusions, evidence level, and claim state when applicable. A
status report without traversable identity is an assertion, not project truth.
