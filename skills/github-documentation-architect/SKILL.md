---
name: github-documentation-architect
description: Architect and govern enterprise software documentation across repository Markdown, mdBook, GitHub Wiki, pull requests, and GitHub Releases. Use for documentation strategy, information architecture, Wiki design, release documentation, migrations, troubleshooting, known issues, API and architecture docs, or documentation quality gates.
---

# GitHub Documentation Architect

Design documentation as a versioned product with explicit audiences, authority, lifecycle, and executable quality gates.

## Start with authority

1. Identify product, audience, task, version, and reader decision.
2. Inventory existing canonical sources before adding a page.
3. Assign exactly one canonical owner for each fact.
4. Choose projections only after canonical location is clear.
5. Define freshness, review, deprecation, and removal rules.
6. Add link, build, example, and release checks appropriate to the content.

Read [information architecture](references/information-architecture.md) before restructuring documentation.

Read [project documentation boundaries](references/project-documentation-boundaries.md)
before adding plans, roadmaps, status, milestone, or evidence pages.

## Alpine surface contract

- Source and root architecture files own implemented truth.
- mdBook owns durable architecture, developer guidance, APIs, quality protocols, use cases, and accepted research narratives.
- GitHub Issues own live requirements, tasks, research status, approval, and blockers.
- GitHub Wiki is a generated, revision-pinned retrieval mirror with no unique claims.
- Pull requests own review-time context, risk, and acceptance evidence.
- GitHub Releases own immutable shipped-version notes, assets, checksums, migrations, compatibility, and known issues.

Do not copy the same prose across surfaces. Summarize and link.

For Alpine, `docs/SUMMARY.md` is mdBook navigation only. It never owns project
status, requirements, milestones, blockers, readiness, or claims. Stable delivery
paths live under `docs/project/`; mutable planning facts remain in GitHub.

## Audience architecture

Serve evaluator, new user, daily user, operator, contributor, maintainer, security reviewer, performance researcher, and release engineer routes explicitly. Landing pages state audience, prerequisites, supported versions, shortest successful path, failure recovery, and canonical deeper source.

## Documentation workflow

1. Write a documentation contract from the accepted issue or release.
2. Classify content as tutorial, how-to, reference, explanation, troubleshooting, decision, research, or release record.
3. Place it in the canonical surface and add audience navigation.
4. Add exact commands, diagrams, examples, and failure cases where they reduce ambiguity.
5. Validate links, examples, navigation, generated Wiki output, and release references.
6. Record supersession instead of silently rewriting historical decisions or research.

## PR metadata preflight

Before opening a documentation pull request, read the repository template and
validate its final Conventional Commit title, template sections, issue and
parent links, release label, base, and source head. Apply the complete title,
body, and required labels in the initial creation command. A later title, body,
label, base, or source-head change creates a new review event: retain the prior
run, mark it superseded rather than erased, and require a new conforming
exact-head aggregate result before describing the change as merge-ready.

## Research lineage and evidence

For architecture, renderer, text, performance, memory, comparator, or product
mechanisms, preserve origin and change history as part of documentation quality.
Read [research lineage](references/research-lineage.md) and
[evidence ledgers](references/evidence-ledgers.md) before changing a case study,
mechanism matrix, comparative claim, or historical conclusion.

- Separate accepted comparator pins from current-upstream review revisions.
- Distinguish adapted concepts, independent convergence, original work,
  comparator-only inputs, rejected scope, and deferred scope.
- Preserve exact source and license boundaries. Never imply copied code without
  a source range, destination range, transformation, license, and review record.
- Record correctness, performance, and memory evidence independently.
- Append supersession history. Do not rewrite an older conclusion to look as if
  it always contained later evidence.
- Retain invalid selectors, zero-selected tests, failed controls, and other
  superseded evidence with the reason they support no claim.
- Distinguish the reviewed source head, any hosted synthetic merge-test
  revision, the final merge revision, and the artifact identity.
- Link every promoted claim to its implementing issue, pull request, evidence
  identity, claim state, and next missing experiment.

## Project-path documentation

Stable path documents state the mission, accepted boundary, dependency graph,
milestone exit semantics, claim rules, and deferred scope. They link live Issues,
Projects, and Milestones but do not copy owners, status, counts, dates, or
percentages. A generated Wiki projects the stable path and live retrieval links;
it is not another planning system.

Read [documentation freshness](references/documentation-freshness.md) before
publishing status-like material or declaring a page current.

Use this reconciliation order: native issue hierarchy and dependencies,
Project projection, stable repository documentation, Wiki source projection,
Wiki publication, then fetched-remote audit. A documentation change never
repairs stale Project state, and a Project mutation never publishes durable
technical truth. When the Project schema changes, update its operating guide
and installed skills in the same bounded task; project kind remains in issue
labels, owner remains in Assignees, and blockers remain native issue edges.

Before describing the live GitHub Wiki as current, run the live remote drift audit
from a clean exact `origin/main` checkout. Local source validation and rendering
prove only the repository templates; they do not prove that the fetched Wiki
remote contains the same bounded pages or source revision.

## Publication state

Report documentation state using the narrowest exact stage: local candidate,
pushed branch, open pull request, merged repository source, published Wiki, or
audited live Wiki. Preserve the commit, branch, pull request, merge revision, and
Wiki revision identities that exist at that stage.

Never collapse these stages. An Issue comment can make a local candidate
discoverable, but it does not publish the files. A green local gate does not
prove hosted CI. A merged repository document does not prove the Wiki was
published. A Wiki push does not prove freshness until the fetched-remote audit
passes from clean exact `origin/main`.

## Wiki design

Use the AWS ParallelCluster Wiki as an information-architecture example, not content to copy. Its useful pattern is task and audience grouping, clear troubleshooting and known-issues routes, and explicit deprecated areas. Alpine improves it by generating every Wiki page from reviewed repository sources and pinning the source revision.

Read [Wiki and release operations](references/wiki-and-releases.md) before publishing or changing either surface.

## Release documentation

Every supported release answers what changed, who is affected, compatibility, install or upgrade path, migrations, security relevance, known issues, rollback, artifact identity, checksums, and defect reporting. Generated notes are input, not a finished release narrative.

## Quality bar

- No orphan pages, broken links, or unversioned version-specific claims.
- No architecture assertion without source or accepted decision.
- No unique operational fact in a generated mirror.
- No command called safe without preconditions and failure behavior.
- No release asset without identity and integrity evidence.
- No deprecated page without replacement or terminal status.
- No documentation promise contradicting tests or shipping behavior.
- No comparative claim without exact evidence identity and a stated ceiling.
- No source lineage change without append-only history and provenance review
  when copied code is alleged.
- No stable project-path page containing manually maintained live status.
- No `SUMMARY.md` entry treated as delivery or milestone authority.
- No live Wiki freshness claim without a successful fetched-remote audit.
- No local candidate or pushed branch described as merged or published.
- No superseded failed or canceled check suite omitted from the publication history.

## Output

Produce the authority matrix, audience map, navigation tree, proposed files,
migration or redirect plan, lineage and evidence impact, validation commands,
freshness trigger, ownership and review cadence, and unresolved decisions.
Distinguish fact, inference, recommendation, implemented, reproduced, and
qualified.
