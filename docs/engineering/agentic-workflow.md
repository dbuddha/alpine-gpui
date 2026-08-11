# Agentic Engineering Workflow

## Research question

What repository workflow best preserves correctness, reviewability,
provenance, and release quality when coding agents perform substantial work?

There is no single authoritative agentic-development standard. Alpine combines
primary standards whose guarantees compose well and records the resulting
workflow as an engineering decision.

## Evidence and decisions

### Hierarchical instructions

`AGENTS.md` is an open, predictable format described as a README for coding
agents. GitHub also documents that multiple files can be stored through a
repository and that the nearest file in the directory tree takes precedence.
Alpine therefore keeps stable cross-cutting rules at the root and adds scoped
files only where architecture, documentation, CI, or change records need more
specific instructions.

- [AGENTS.md open format](https://github.com/agentsmd/agents.md)
- [GitHub repository instruction hierarchy](https://docs.github.com/en/copilot/how-tos/configure-custom-instructions-in-your-ide/add-repository-instructions-in-your-ide)

Large generic instruction dumps can distract from local work. Alpine avoids
duplicating root rules in every directory and links to durable source-of-truth
documents instead.

### Commits and pull requests

[Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/)
provides a machine-readable subject without replacing a human explanation.
Alpine uses its `type(scope): summary` form for branch commits and PR titles.

GitHub recommends squash merging when a pull request represents one logical
change and contains intermediate work commits. Alpine treats the PR as the
review and evidence envelope, then squash-merges one coherent commit into a
linear `main` history.

- [GitHub pull request merge strategies](https://docs.github.com/en/pull-requests/reference/pull-request-merges)
- [GitHub squash merge configuration](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/configuring-commit-squashing-for-pull-requests)

The PR template requires context, decision or root cause, evidence, risk,
tests, performance impact, provenance, and change-record status. Agent process
narration is not evidence and does not belong in the commit or changelog.

### Protected integration

GitHub branch protection can require pull requests, latest-SHA status checks,
conversation resolution, linear history, signed commits, and no bypass.
Alpine currently requires a strict `ci-pass`, pull requests, linear history,
conversation resolution, administrator enforcement, and no force pushes or
deletion. Required human approvals remain zero while there is only one owner,
because authors cannot provide independent approval for their own PR.

- [GitHub protected branches](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches)
- [GitHub required status checks](https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/troubleshooting-required-status-checks)

Signed commits are desirable but are not enabled until the owner's local and
automation signing identities are configured and tested without blocking safe
recovery.

### Changelog fragments

Towncrier documents the core advantage of news fragments: contributors write
small user-facing files instead of competing over a single unreleased section,
and release tooling later assembles the digest. Alpine adopts the pattern
without adding Towncrier as a dependency.

- [Towncrier philosophy](https://towncrier.readthedocs.io/en/stable/)
- [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/)
- [Semantic Versioning 2.0.0](https://semver.org/)

Per-PR fragments live in `changes/`. At release, they are curated into the
root `CHANGELOG.md` under Added, Changed, Deprecated, Removed, Fixed,
Performance, and Security headings. GitHub Releases repeat that durable file;
they do not replace it.

Alpine remains pre-1.0 until it declares a public API. SemVer explicitly treats
major version zero as initial development.

### CI and supply chain

GitHub states that a full commit SHA is the only immutable way to reference an
Action. Alpine pins Actions to full SHAs, uses read-only workflow permissions,
allows only GitHub-owned Actions, and tests the committed Cargo lockfile.

- [GitHub Actions secure-use reference](https://docs.github.com/en/actions/reference/security/secure-use)
- [Repository Actions policy](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/enabling-features-for-your-repository/managing-github-actions-settings-for-a-repository)

OpenSSF Scorecard treats branch protection, code review, CI tests, and pinned
dependencies as visible supply-chain controls. Alpine uses those categories as
an audit checklist even though the repository is private.

- [OpenSSF Scorecard checks](https://github.com/ossf/scorecard/blob/main/docs/checks.md)

Release artifacts will eventually use GitHub artifact attestations, which bind
build provenance to repository, commit, workflow, and triggering event.

- [GitHub artifact attestations](https://docs.github.com/en/actions/concepts/security/artifact-attestations)

## Alpine change protocol

1. Recover context from root and scoped instructions plus durable decisions.
2. State objective, excluded scope, risks, owner decisions, and acceptance gate.
3. Create one short-lived branch from current `origin/main`.
4. Implement the narrowest complete vertical slice.
5. Add tests, documentation, provenance, ADRs, and change fragments as needed.
6. Run focused checks followed by `scripts/check.sh`.
7. Inspect status, staged and unstaged diffs, untracked files, and recent style.
8. Commit with Conventional Commit form.
9. Ask the owner before pushing or creating the PR.
10. Complete the PR template with reproducible evidence.
11. Require strict latest-SHA CI and resolve every conversation.
12. Squash-merge one logical commit and delete the branch.
13. Curate fragments into `CHANGELOG.md` only during a release PR.

## Review standard for agent-authored work

Judge artifacts, not confidence. A PR is acceptable only when:

- its scope can be stated in one sentence;
- the diff contains no unrelated cleanup;
- ownership and failure behavior are explicit;
- tests would fail for the important prior defect or missing behavior;
- performance claims have raw reproducible evidence;
- provenance and dependency decisions are auditable;
- generated or automated changes are deterministic and reviewed;
- CI passes on the exact commit to merge;
- rollback or disablement is understood for risky changes.

## Future repository-setting upgrades

After owner approval:

- allow squash merge only and use the PR title as the squash subject;
- delete branches automatically after merge;
- require signed commits after signing identities are qualified;
- use a ruleset if it provides clearer auditability than the current branch
  protection without weakening it;
- add artifact attestations, SBOM generation, and release environments when
  distribution begins.
