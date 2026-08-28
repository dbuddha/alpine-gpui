# GitHub agent operations

Alpine owns three installable skills that turn repository policy into repeatable agent workflows. They advise and validate work; they do not supersede `AGENTS.md`, approved issues, CI, or owner-held approvals.

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
5. Inspect all three installed link targets before removing the old worktree.

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
