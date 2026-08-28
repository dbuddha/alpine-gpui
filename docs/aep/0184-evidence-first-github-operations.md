# AEP 0184: Evidence-first GitHub operations

- Status: Accepted
- Requirement: [#184](https://github.com/dbuddha/alpine-gpui/issues/184)
- Capability: [#28](https://github.com/dbuddha/alpine-gpui/issues/28)
- Corrective defect: [#398](https://github.com/dbuddha/alpine-gpui/issues/398)
- Implementing tasks: [#185](https://github.com/dbuddha/alpine-gpui/issues/185),
  [#225](https://github.com/dbuddha/alpine-gpui/issues/225),
  [#317](https://github.com/dbuddha/alpine-gpui/issues/317),
  [#319](https://github.com/dbuddha/alpine-gpui/issues/319), and
  [#357](https://github.com/dbuddha/alpine-gpui/issues/357)

## Context

Alpine uses GitHub Issues, pull requests, milestones, Projects, repository
Markdown, mdBook, the Wiki, Releases, and repository-owned agent skills as one
delivery system. These surfaces have different owners: Issues and Projects own
live execution state, repository files own versioned technical truth, the Wiki
is a generated retrieval mirror, and Releases own immutable distribution facts.
Conflating them creates stale status, unreviewed decisions, or evidence that
exists only in a local worktree.

Requirement #184 established this operating boundary after the original
assurance bootstrap. Its implementation had deterministic tests but no atomic
claim registration. Closing child Task #357 therefore failed the production
hierarchy gate even though its implementation passed `ci-pass`.

## Decision

Repository-owned GitHub skills and automation remain non-shipping tools with
explicit authority boundaries. Every external mutation starts from a read-only
inventory, fails closed on ambiguous ownership or missing access, and records
the resulting GitHub identity. Local fixtures validate behavior without making
ordinary CI depend on live Project or Wiki access.

Repository Markdown and mdBook remain canonical for versioned architecture,
protocols, research, and decisions. The Wiki is generated and audited against
that source. `docs/SUMMARY.md` remains navigation only. GitHub Issues and, when
the token permits, Projects own live priority, owner, blocker, and completion
state. A missing `read:project` scope makes issue hierarchy authoritative; it
does not make an inaccessible Project empty.

## Atomic claims

- **AEP-0184-C01:** Repository-owned agent skills validate their metadata and
  references, install only links they own, refuse unrelated overwrites, and can
  verify installation without mutation.
- **AEP-0184-C02:** Wiki publication distinguishes local template validity from
  live remote synchronization and detects missing, unknown, stale, mixed, or
  byte-divergent pages without changing the Wiki worktree.
- **AEP-0184-C03:** Research retention rejects missing source identities,
  evidence levels, requirement anchors, comparator assumptions, or canonical
  retrieval links before a research package is accepted.
- **AEP-0184-C04:** Issue hierarchy and worktree operations fail closed on
  missing approval, parent, evidence registration, active pull requests, dirty
  state, unique unarchived commits, missing registrations, or an excessive
  worktree count.

## Evidence contract

Deterministic shell integration fixtures exercise successful and rejected
skill installation, Wiki drift states, research retention, issue hierarchy,
and worktree classification. The production hierarchy workflow must continue
to reject an approved Requirement with no registered assurance claim. A
registered Requirement must pass that precondition before parent completion is
considered.

Kani, TLA+, Miri, native GPU validation, and fixed-hardware timing are not
applicable to these claims. The implementations coordinate repository files,
Git processes, and GitHub API state rather than a bounded shipping transition
system. Any future extracted pure scheduler receives a separate applicability
review.

## Failure and recovery

Missing permissions, stale remote state, malformed metadata, conflicting
authority, unknown worktree ownership, or absent evidence returns a nonzero
status with an actionable diagnostic. Recovery updates canonical state or its
mapping and reruns the failed operation. It never bypasses a guard, force-pushes
history, deletes unique work, or treats local files as published GitHub state.

## Reversal conditions

Revisit this decision only if GitHub changes the authoritative hierarchy or
Wiki APIs, repository scale makes deterministic local fixtures materially
insufficient, or a measured delivery bottleneck requires a different canonical
owner. Do not create another project database, handwritten Wiki fork, or
shipping runtime dependency to solve an operations concern.
