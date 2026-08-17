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

## Status and field defaults

Use `Backlog`, `Ready`, `In Progress`, `Review`, `Blocked`, and `Done` unless the repository already owns a compatible workflow. Do not encode status again in labels.

Prefer typed fields `Priority`, `Risk`, `Milestone`, `Estimate`, `Confidence`, `Horizon`, `Target`, `Parent`, and `Blocked by` when Project access exists. Use stable estimates `1`, `2`, `3`, `5`, and `8`; they express relative delivery complexity, not hours. A field without a decision or view consumer is dead metadata.

## Planning algorithm

1. Expand the capability to approved requirements and leaf tasks.
2. Mark prerequisites with native dependency edges.
3. Find the longest unresolved dependency chain and all blockers on it.
4. Assign milestones to leaf work, then derive parent progress from leaves.
5. Put only unblocked, accepted work in Ready.
6. Limit In Progress to work with an active owner or agent and current branch or evidence run.
7. Put a task in Review only when a PR or evidence artifact exists.
8. Move to Done only when closure semantics and checks are satisfied.

For Alpine, correctness is the first ordering key, then performance, resource efficiency, and delivery speed. Never accelerate delivery by weakening an earlier key.

## Metrics and forecasts

Use burn-up as the primary historical chart because it exposes completed work and scope growth. Use burn-down only against an explicitly frozen scope snapshot. Pair either chart with scope trend, leaf completion, throughput, cycle time, blocker age, and critical-path status.

Read [metrics and reporting](references/metrics-and-reporting.md) before publishing progress, forecasts, or milestone health.

## Mutation safety

- Prefer stable node IDs for GraphQL mutations.
- Preflight permissions and current field option IDs.
- Make idempotent changes and verify each remote result.
- Do not create duplicate issues to compensate for missing Project access.
- Fall back to issue hierarchy when Project permissions are absent.
- Never force-push, delete branches, bypass checks, fabricate dates, or mark inconclusive evidence green.
- Ask immediately before a push or release when repository policy requires it.

## Alpine completion report

Report exact capability and requirement, completed leaf tasks, open critical-path tasks, blockers with age and owner, milestone exit criteria, PR and check state, scope changes, and the next smallest uncompromised slice. Distinguish fact, inference, and recommendation.
