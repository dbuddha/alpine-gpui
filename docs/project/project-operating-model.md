# GitHub project operating model

Alpine uses GitHub as an evidence-linked execution control plane. It does not
maintain a second task database in mdBook or the Wiki.

## Canonical ownership

| Fact | Canonical surface |
| --- | --- |
| End-to-end observable outcome | Capability issue |
| Approved testable behavior | Requirement issue |
| Independently reviewable work | Task or Defect issue |
| Investigation question and disposition | Research issue |
| Reproducible protocol and result | Experiment issue |
| Direct decomposition | Sub-issue relationship |
| Required ordering | Blocked-by relationship |
| Live priority, owner, status, and blocker projection | [Project #1](https://github.com/users/dbuddha/projects/1) |
| Outcome cohort | GitHub Milestone |
| Proposed revision and review evidence | Pull request |
| Executable revision evidence | CI check and retained artifact |
| Stable engineering truth | Source, `ARCHITECTURE.md`, and mdBook |
| Human retrieval | Generated revision-pinned Wiki |
| Shipped truth | Signed tag and GitHub Release |

When Project #1 cannot be read, use issues, sub-issues, dependencies, milestones,
pull requests, and checks as the fallback. Never infer that inaccessible fields
are empty, create duplicate issues, or copy temporary status into documentation.

## Hierarchy and closure

- A capability closes only when its required approved outcomes are accepted.
- A requirement closes only when its required leaf work and end-to-end evidence
  satisfy the acceptance contract.
- A task closes after its bounded implementation or evidence result is merged
  and accepted.
- A defect closes after reproduction, correction, and a regression protecting
  the observed behavior.
- Research closes when the reviewed repository package records its sources,
  findings, limits, decisions, and follow-up links.
- An experiment remains separate from research and closes only with protocol,
  implementation, raw evidence, analysis, and conclusion.
- A merged pull request never silently closes unperformed physical or
  comparative qualification.

Each task has one direct parent. Related requirements use links. A dependency
edge means the downstream contract cannot be satisfied first; a sequencing
preference is not a blocker.

## Preferred Project fields

Use fields only when a view or decision consumes them:

- Type.
- Gate.
- Status.
- Priority.
- Risk.
- Blocked By.
- Evidence Level.
- Claim State.
- Estimate.
- Acceptance Gate.

Preferred workflow states are `Proposed`, `Ready`, `In Progress`, `Blocked`,
`In Review`, and `Done` when the existing Project supports them. Labels classify
work and risk; they do not duplicate workflow state.

## Required retrieval views

- Private Dogfood Critical Path.
- M4 Physical Qualification.
- M5 Daily Driver.
- Performance and Memory.
- Evidence Debt.
- Research Requalification.
- Deferred Scope.
- Release Readiness.
- Stale Truth Reconciliation.

A named view is a desired retrieval contract, not proof that the current token
can inspect or mutate it. Issue-first saved queries are the fallback.

## Planning and metrics

Build the critical path from unresolved dependency edges between accepted leaf
tasks. Put work in `Ready` only when it is approved, unblocked, bounded, and has
an acceptance gate. `In Progress` requires an active owner and current branch or
evidence run. `In Review` requires a pull request or evidence artifact. `Done`
requires closure semantics, not a manual field move.

Report:

- Accepted leaf-task burn-up.
- Scope growth against the same accepted boundary.
- Throughput and cycle time.
- Blocker age and owner.
- Longest unresolved dependency chain.
- Milestone exit criteria remaining.
- Evidence level and claim state.

Use burn-down only after a scope snapshot is explicitly frozen. Do not calculate
readiness from parent issue totals, issue volume, research pages, source files,
or merged pull requests without accepted leaves.

## Reconciliation loop

1. Capture repository, branch, revision, dirty state, permissions, hierarchy,
   milestones, Project visibility, pull requests, and required checks.
2. Reconcile issue type and direct parent.
3. Reconcile requirement approval before implementation.
4. Reconcile dependency edges and current blockers.
5. Reconcile milestone membership on leaf work.
6. Reconcile Project fields when readable.
7. Detect merged implementation pull requests whose tasks remain ambiguously
   open, and closed tasks whose required evidence is absent.
8. Reconcile mdBook and Wiki links only after canonical GitHub truth is sound.
9. Report exact changes, missing permissions, unresolved blockers, and the next
   smallest uncompromised leaf.

## Pull request evidence

Every pull request links its task and parent outcome and records Context, Root
Cause when relevant, Evidence, Risk and Scope, and Test Plan. The exact tested
head must be the head that is merged. Each comparative or performance result
also links its workload, environment, raw samples, exclusions, claim state, and
lineage history.
