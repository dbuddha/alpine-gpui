---
name: github-project-operator
description: Operate GitHub Issues, sub-issues, dependencies, pull requests, Projects, milestones, blockers, critical paths, and delivery metrics as one evidence-first system. Use when creating or reconciling plans, tickets, project fields, milestones, status reports, burn-up or burn-down charts, blockers, or release readiness.
---

# GitHub Project Operator

Treat GitHub as an execution control plane, not a task graveyard. Preserve one canonical fact per surface and make every status claim traversable to evidence.

## Operating contract

1. Restate the objective, acceptance gate, and human-owned decisions.
2. Identify repository, branch, dirty state, permissions, issue hierarchy, active milestones, Project configuration, and required checks.
3. Capture a read-only snapshot before proposing a mutation.
4. Reconcile canonical entities before creating anything new.
5. Present intended mutations and consequences as a dry run.
6. Apply only approved mutations, in dependency order, using stable identifiers.
7. Re-read affected remote state and report exact deltas, unresolved blockers, and next critical-path work.

Never infer completion from prose, a parent counter, a merged branch, or an item moved to Done. Completion requires accepted evidence and closure rules.

## Canonical ownership

- Capability: end-to-end observable outcome.
- Requirement: approved, testable product or technical behavior.
- Task: bounded implementation or evidence-producing unit.
- Sub-issue: decomposition with one direct parent.
- Dependency: explicit blocked-by or blocking edge.
- Pull request: proposed revision plus acceptance evidence.
- Project: views, typed metadata, planning state, and historical charts.
- Milestone: acceptance-gated outcome cohort, not a speculative date bucket.
- CI and release: executable evidence and shipped truth.

Read [the operating model](references/operating-model.md) before changing hierarchy, fields, statuses, milestones, or views.

Read [evidence-aware delivery](references/evidence-aware-delivery.md) before
closing work or reporting completion. Read
[qualification projects](references/qualification-projects.md) before planning
performance, memory, hardware, or comparator claims.

## Status and field defaults

Use the repository's existing workflow. Alpine prefers `Proposed`, `Ready`,
`In Progress`, `Blocked`, `In Review`, and `Done` when Project configuration
supports them. Do not encode status again in labels.

Prefer typed fields `Type`, `Gate`, `Priority`, `Risk`, `Blocked By`, `Evidence
Level`, `Claim State`, `Estimate`, and `Acceptance Gate` when Project access
exists. Use stable estimates `1`, `2`, `3`, `5`, and `8`; they express relative
delivery complexity, not hours. A field without a decision or view consumer is
dead metadata.

## Planning algorithm

1. Expand the capability to approved requirements and leaf tasks.
2. Mark prerequisites with native dependency edges.
3. Find the longest unresolved dependency chain and all blockers on it.
4. Assign milestones to leaf work, then derive parent progress from leaves.
5. Put only unblocked, accepted work in Ready.
6. Limit In Progress to work with an active owner or agent and current branch or evidence run.
7. Put a task in Review only when a PR or evidence artifact exists.
8. Move to Done only when closure semantics and checks are satisfied.

Separate implementation tasks from qualification tasks. A merged mechanism may
be implemented while physical, residency, statistical, or comparative evidence
remains open. Separate Research closure from Experiment closure. Read
[research-to-experiment handoff](references/research-to-experiment-handoff.md)
before converting findings into measured work.

## Reconciliation and critical path

Before creating work, detect duplicate issues, stale Project fields, merged
implementation pull requests whose tasks remain open, closed tasks missing named
evidence, parents counted as delivery, and deferred work leaking into active
milestones. Read [critical-path reconciliation](references/critical-path-reconciliation.md).

Build milestone dependency graphs from accepted outcome prerequisites rather
than milestone numbering. Protect deferred paths from entering the current
critical path without a new approved requirement.

For Alpine, correctness is the first ordering key, then performance, resource efficiency, and delivery speed. Never accelerate delivery by weakening an earlier key.

## Metrics and forecasts

Use accepted-leaf burn-up as the primary historical chart because it exposes
completed work and scope growth. Use burn-down only against an explicitly frozen
scope snapshot. Pair either chart with scope trend, accepted leaf completion,
throughput, cycle time, blocker age, critical-path status, Evidence Level, and
Claim State.

Read [metrics and reporting](references/metrics-and-reporting.md) before publishing progress, forecasts, or milestone health.

## Mutation safety

- Prefer stable node IDs for GraphQL mutations.
- Preflight permissions and current field option IDs.
- Make idempotent changes and verify each remote result.
- Do not create duplicate issues to compensate for missing Project access.
- Fall back to issue hierarchy when Project permissions are absent.
- Treat inaccessible Project fields as unknown, never empty.
- Never promote Evidence Level or Claim State without exact retained identity.
- Never force-push, delete branches, bypass checks, fabricate dates, or mark inconclusive evidence green.
- Ask immediately before a push or release when repository policy requires it.

## Alpine completion report

Report exact capability and requirement, accepted leaves, open critical-path
tasks, blockers with age and owner, milestone exit criteria, PR and exact-head
check state, scope growth, Evidence Level, Claim State, and the next smallest
uncompromised slice. Distinguish fact, inference, recommendation, implemented,
reproduced, and qualified.
