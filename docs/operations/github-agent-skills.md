> Historical reference. Superseded by [AGENTS.md](../../AGENTS.md).
> The governance and skill evaluation procedures below are retired.

# GitHub agent operations

Alpine owns manifest-listed installable skills that turn repository policy into repeatable agent workflows. The three GitHub authorities below route to the focused [engineering skills](alpine-engineering-skills.md) when needed. They advise and validate work; they do not supersede `AGENTS.md`, approved issues, CI, or owner-held approvals.

| Skill | Use it for | Canonical output |
| --- | --- | --- |
| `github-project-operator` | Issues, dependencies, Projects, milestones, blockers, critical path, burn-up, and status | Reconciled GitHub state linked to evidence |
| `github-documentation-architect` | mdBook, Wiki, architecture and API docs, troubleshooting, known issues, Releases | One authority per fact plus tested projections |
| `github-deep-researcher` | Source archaeology, papers, experiments, comparator research, agent evaluation | Source-pinned package and implementing decisions |

## Install

```sh
scripts/install-agent-skills.sh --install
scripts/install-agent-skills.sh --check
```

The installer creates absolute symbolic links under `${CODEX_HOME:-$HOME/.codex}/skills`. It preflights all destinations before creating or removing links and refuses paths owned by another source.

```sh
scripts/install-agent-skills.sh --remove-links
```

## Authority boundaries

Issues own live work and approval. Projects own views and planning metadata. Repository Markdown and mdBook own durable technical truth. Wiki is generated navigation. Releases own shipped-version facts and assets. Skills cannot lower repository gates.

Wiki source checks are deliberately offline. They prove templates, manifests,
links, and deterministic rendering but not the content currently published on
GitHub. Before reporting the live Wiki as current, use a clean exact `main`
checkout and run:

```sh
scripts/wiki.sh audit-remote /path/to/alpine-gpui-wiki
```

The audit fetches and compares without a commit or push path. Failed remote
access means freshness is unknown.

The [project execution package](../project/README.md) owns the stable
daily-driver path, milestone gate semantics, operating model, claim readiness,
and deferred scope. The [lineage package](../research/alpine-lineage/index.md)
owns mechanism origin, Alpine modifications, historical supersession, and
evidence ceilings. `docs/SUMMARY.md` only exposes these pages in mdBook
navigation.

## Publication-state reporting

Name the exact state when bridging local work to GitHub: local commit, pushed
branch, open pull request, merged repository source, published Wiki, or audited
live Wiki. Include the corresponding commit, pull request, merge, and Wiki
revision identities when they exist. An Issue comment can retain the location
of a local candidate, but it does not make that candidate published. Likewise,
merged templates do not make the live Wiki current until publication and the
fetched-remote audit both succeed.

## Project access preflight

Before reading or changing a GitHub Project, verify the active token can list
the intended Project and read its fields. A missing `read:project` scope means
Project status, priority, blockers, estimates, and charts are unknown, not
empty. In that state:

- keep capabilities, requirements, tasks, sub-issues, dependencies, pull
  requests, milestones, and CI evidence authoritative;
- do not infer Project values or create duplicate issues to compensate;
- report the access limitation in status output;
- defer Project-only mutations until an authorized token is available.

Burn-up is the default progress chart once Project history is readable. Use
burn-down only for an explicitly frozen scope snapshot, and always pair either
chart with scope change, leaf completion, cycle time, blocker age, and the
current critical path.

## Live Project schema

Project #1 does not duplicate GitHub-native issue facts. `kind:*` labels own
work type, Assignees own delivery ownership, Milestones own outcome cohorts,
parent and sub-issue relationships own decomposition, and native blocked-by
relationships own required ordering.

Custom fields own only delivery projection: `Delivery Gate`, `Evidence Level`,
`Workload`, and `Acceptance Gate`. The board also retains `Priority`, `Risk`,
`Horizon`, `Remaining weeks`, and `Confidence`. Status values are `Backlog`,
`Ready`, `In Progress`, `Review`, `Blocked`, and `Done`. Do not create custom
`Type`, `Owner`, or `Blocked By` fields alongside the native authorities.

If an environment `GH_TOKEN` cannot read Project #1, inspect `gh auth status`
before declaring Project state unknown. An authenticated keyring credential may
have different scopes; a read-only retry with that credential is allowed only
after its identity and authorization are verified. This is credential
selection, not a check bypass.

Reconcile in this order: issue hierarchy and native dependencies, Project
projection, stable repository documentation, generated Wiki source, published
Wiki, and fetched-remote Wiki audit. `docs/SUMMARY.md` remains navigation only.

## Pull request creation preflight

Before creating a pull request, read the repository template and validate the
final Conventional Commit title, complete body, closing issue and parent chain,
release label, base, and source head as one metadata snapshot. Apply the title,
body, and required labels in the initial creation command. This prevents a
known-invalid metadata event from starting an expensive CI matrix. Alpine CI
starts a new pull request from the settled release-label event rather than
`opened`; an unlabeled pull request remains blocked until that event exists.

If any of those fields changes after checks start, retain the earlier run as
superseded evidence and require a new exact-head aggregate result. Never hide a
failed or canceled run, merge from a stale source head, or treat a successful
older suite as evidence for corrected metadata.

A canceled and a successful required check on the same current pull-request
state is a CI trust defect, not a mergeable green result. Preserve both run IDs,
refuse administrator bypass, obtain one clean later metadata event, and correct
the trigger or concurrency policy. Metadata events must not cancel another
required check at the same SHA; source updates may cancel checks on obsolete
SHAs.

## Merge target and evidence preflight

`--auto` is not a promise to wait. Use auto-merge only against the protected
default branch, never an intermediate stacked base, even when that stack has
some protection. An unprotected target can merge immediately. Manual stacked
merges still require terminal-green applicable exact-head checks and `ci-pass`.

Before every merge, fetch the base and retain the live PR source SHA, base SHA,
effective classic protection and rulesets, required check identities and their
applicability, selected check count, hosted run metadata, and mergeability.
Establish the base actually tested by that run; do not substitute the current
base when the run does not identify it. Unknown or conflicting state blocks
the merge. The source and tested base must match the current candidate and base.

The installed project operator uses this repository-owned snapshot policy:

```sh
scripts/check-agent-skills.sh --merge-readiness \
  MODE DEFAULT_BRANCH BASE_BRANCH PROTECTION \
  EXPECTED_HEAD RUN_HEAD EXPECTED_BASE TESTED_BASE \
  AGGREGATE REQUIRED_CHECKS SELECTED_COUNT MERGEABILITY
```

`MODE` is `manual` or `auto`; `PROTECTION` is `protected` or `unprotected`.
Revision arguments are full lowercase nonzero SHA-1 identities. `AGGREGATE` and
`REQUIRED_CHECKS` must both be `success`, `SELECTED_COUNT` must be a positive
canonical integer at most 9999, and `MERGEABILITY` must be `mergeable`. Required
check applicability is resolved from GitHub policy, not inferred from a count.
Retain the underlying responses as evidence rather than only these summaries.

This checker consumes caller-supplied values. It does not contact GitHub,
authenticate evidence, discover tests, grant permission, lock refs, or make a
merge race-proof. Recheck the live snapshot immediately before an allowed,
source-safe merge. A source or base change invalidates the earlier decision.
After a stacked merge, require fresh post-stack acceptance for the main PR;
after a main merge, require exact-main CI before advancing dependent work.
Never repair the evidence trail by erasing canceled or failed runs.

### Retained incident and safe supersession

[Defect #520](https://github.com/dbuddha/alpine-gpui/issues/520) records
[PR #519](https://github.com/dbuddha/alpine-gpui/pull/519) merging into an
unprotected stack while 16 checks were pending. Its intermediate commit remains
historical evidence, not an accepted main revision.
[PR #517](https://github.com/dbuddha/alpine-gpui/pull/517) received a fresh
post-stack run, failed, and closed unmerged. The isolated replacement
[PR #536](https://github.com/dbuddha/alpine-gpui/pull/536) merged only after its
exact-head aggregate completed. This is safe supersession, not a retroactive
green result for #517 or acceptance of every change in the old stack.

[PR #582](https://github.com/dbuddha/alpine-gpui/pull/582) added the executable
policy, rejection controls, and installed skill guidance. The
[retained receipt](https://github.com/dbuddha/alpine-gpui/issues/520#issuecomment-5564601665)
binds those changes to source, tested base, runs, and the historical disposition.
Deterministic policy tests do not establish improved agent decision quality;
[Experiment #566](https://github.com/dbuddha/alpine-gpui/issues/566) must evaluate
that separately with tools and permissions held constant between skill variants.

## Research depth

Deep research states a decision question, pins primary sources, separates facts from inference, seeks contradictory evidence, records validity threats, reproduces consequential behavior, and links findings to requirements. Architecture adoption requires E2, performance design claims E3, and dominance claims E4.

Substantial research uses a frontmatter index plus source map, findings, experiments, decisions, bibliography, and checksummed raw evidence. Alpine's current path policy must change through an approved task before a new package layout is introduced.

## Migrating installed links between Alpine worktrees

The installer refuses to replace a link owned by another checkout. Migrate
without bypassing that protection:

1. Confirm the old checkout is an Alpine worktree and its skill content has no
   unique unmerged changes.
2. Run `scripts/install-agent-skills.sh --remove-links` from the old checkout.
3. Run `scripts/install-agent-skills.sh --install` from clean current Alpine
   `main`.
4. Run `scripts/install-agent-skills.sh --check` from current `main`.
5. Inspect every manifest-listed installed link target before removing the old worktree.

Do not unlink or replace a foreign skill path and do not remove a worktree while
installed skills still reference it.

Use the [worktree inventory and cleanup guide](worktrees.md) before creating or
retiring a checkout. `scripts/check-worktrees.sh --plan-remove PATH` is a
read-only safety decision, not a removal command. It refuses dirty, detached,
active-PR, unknown-PR, missing, ambiguous, and unarchived unique candidates.

## Validation

```sh
scripts/check-agent-skills.sh
scripts/test-agent-skills.sh
scripts/check.sh
```

Checks cover metadata, resources, triggers, overwrite refusal, idempotency,
owned-link removal, foreign-link preservation, mdBook navigation, local Wiki
integrity, and deterministic fake-remote Wiki drift detection.
