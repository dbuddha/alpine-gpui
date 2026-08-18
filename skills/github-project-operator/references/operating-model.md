# GitHub operating model

## Source-of-truth matrix

| Fact | Canonical surface | Projection surfaces |
| --- | --- | --- |
| Outcome | Capability issue | Project roadmap, Wiki status page |
| Accepted behavior | Requirement issue | mdBook use case, Project view |
| Bounded work | Task issue | Project board, milestone |
| Ordering | Sub-issue and dependency edges | Critical-path view |
| Proposed revision | Pull request | Task timeline, Project item |
| Executable proof | CI check and artifact | PR evidence section |
| Durable engineering truth | Repository Markdown and code | mdBook, generated Wiki |
| Shipped version | Signed tag and GitHub Release | Wiki release index |

A projection may summarize canonical facts but must link back and must not become an independent authority.

## Snapshot before mutation

Record repository identity, default branch, local branch and dirty state, base and head revisions, token scopes, open capability tree, issue labels and types, milestones, Project number and owner, Project fields and option IDs, active views when discoverable, open pull requests, required checks, and release state. If incomplete, report the missing dimension. Do not treat an inaccessible Project as empty.

## Hierarchy rules

- One capability describes an observable end state.
- One requirement owns one coherent accepted behavior boundary.
- One task is independently reviewable and closes through one PR or one evidence result.
- A task has exactly one direct parent. Related concerns use links, not extra parents.
- A dependency means work cannot satisfy its contract before another item; sequence preferences are not blockers.
- Parent progress is calculated from accepted leaf work, not manually declared.

## Milestone rules

A milestone needs a name, outcome statement, inclusion rule, exclusion rule, entry criteria, exit evidence, and leaf-task assignment. Dates are optional. A milestone with only parent issues gives misleading completion percentages.

Before closing one, audit open leaves, merged PRs without closed tasks, closed tasks without evidence, excluded work that leaked in, unresolved blockers, and release or qualification artifacts.

## Project views

| View | Purpose |
| --- | --- |
| Critical path | Unresolved blockers and dependent leaf tasks |
| Ready queue | Approved, unblocked, acceptance-defined work |
| Active delivery | In Progress and Review with owner, PR, and checks |
| Milestone qualification | Leaf tasks grouped by milestone and risk |
| Research and decisions | Investigations awaiting disposition |
| Recently done | Completed leaves with closure evidence |

## Reconciliation order

1. Validate issue types and direct parents.
2. Validate requirement approval before implementation.
3. Validate native dependency edges.
4. Validate milestone assignment on leaves.
5. Validate Project membership and field values.
6. Validate task, PR, check, and closure consistency.
7. Validate Wiki and mdBook links after canonical state is sound.

## Primary sources

- GitHub Issues and Projects: https://docs.github.com/en/issues/tracking-your-work-with-issues/learning-about-issues/about-issues
- Hierarchy fields in Projects: https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/browsing-sub-issues
- Project issue fields and limits: https://docs.github.com/en/issues/planning-and-tracking-with-projects/understanding-fields/about-issue-fields
