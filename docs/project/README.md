# Project execution documentation

This section explains the stable path from Alpine's mission to a private
daily-driver editor and qualified renderer claims. It does not duplicate live
issue status, Project field values, milestone counts, or CI results.

## Start here

- [Daily-driver path](daily-driver-path.md): overall goal, accepted product
  boundary, dependency graph, critical sequence, and private-dogfood exit gate.
- [Milestone gates](milestone-gates.md): stable outcome, entry, exclusion, and
  exit-evidence definitions for M0 through M7.
- [Project operating model](project-operating-model.md): authority, hierarchy,
  status, blocker, metric, and reconciliation rules.
- [Claim readiness](claim-readiness.md): evidence ceilings and qualification
  rules for correctness, latency, GPU, CPU, and memory statements.
- [Deferred scope](deferred-scope.md): work that cannot expand the macOS
  daily-driver critical path without a newly accepted requirement.

## Authority

| Question | Stable owner | Live or revision owner |
| --- | --- | --- |
| Why does Alpine exist? | [Vision](../vision.md) | Accepted capability issues |
| What must Alpine Studio do? | [Product contract](../use-cases/alpine-studio-highfidelity.md) | Requirement issues |
| How do accepted outcomes compose? | [Daily-driver path](daily-driver-path.md) | Issues and dependencies |
| What closes a milestone? | [Milestone gates](milestone-gates.md) | GitHub Milestones and leaf tasks |
| What is next or blocked? | Operating rules in this section | [Project #1](https://github.com/users/dbuddha/projects/1) when readable, otherwise issue hierarchy |
| What is implemented? | Source and `ARCHITECTURE.md` | Merged pull request revision |
| Where did a mechanism come from? | [Lineage package](../research/alpine-lineage/index.md) | Research issue and implementing pull request |
| What may Alpine claim? | [Claim readiness](claim-readiness.md) | Evidence registry, raw artifacts, CI, and release report |
| What shipped? | Release policy and documentation | Signed tag and GitHub Release |

`docs/SUMMARY.md` is the mdBook navigation tree. It is not a project status,
requirements, milestone, blocker, or readiness database.

## Change rule

Change stable path documentation when the accepted product boundary, dependency
graph, milestone contract, authority model, or evidence policy changes. Change
GitHub issues and Project fields when live ownership, status, priority, or
blocking changes. A generated Wiki publication may summarize and link both, but
it may not become the only copy of either.
