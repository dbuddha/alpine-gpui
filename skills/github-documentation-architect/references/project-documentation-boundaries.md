# Project documentation boundaries

## Stable repository documents

Repository Markdown owns mission, product contract, stable delivery path,
milestone exit semantics, operating model, claim rules, deferred scope,
architecture, research, and qualification protocols.

Stable documents may link live work but must not manually maintain owner,
priority, blocker, status, issue count, readiness percentage, or due date.

## Live GitHub state

- Capabilities own observable outcomes.
- Requirements own accepted behavior.
- Tasks, Defects, Research, and Experiments own live work and acceptance.
- Dependencies own required ordering.
- Projects own live planning projection.
- Milestones own outcome cohorts.
- Pull requests and CI own revision evidence.
- Releases own shipped truth.

If Project access is unavailable, use issue hierarchy and dependencies. Do not
copy board state into mdBook or create duplicate issues.

## Navigation and Wiki

`docs/SUMMARY.md` is navigation only. A generated Wiki is a revision-pinned
retrieval mirror. Neither owns requirements, status, architecture, research,
evidence, decisions, or release instructions.

An execution-map Wiki page should summarize the stable dependency graph and
link the live Project, milestones, and critical issues. It must not copy counts,
percentages, owners, status, or forecasts.
