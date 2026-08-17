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

## Alpine surface contract

- Source and root architecture files own implemented truth.
- mdBook owns durable architecture, developer guidance, APIs, quality protocols, use cases, and accepted research narratives.
- GitHub Issues own live requirements, tasks, research status, approval, and blockers.
- GitHub Wiki is a generated, revision-pinned retrieval mirror with no unique claims.
- Pull requests own review-time context, risk, and acceptance evidence.
- GitHub Releases own immutable shipped-version notes, assets, checksums, migrations, compatibility, and known issues.

Do not copy the same prose across surfaces. Summarize and link.

## Audience architecture

Serve evaluator, new user, daily user, operator, contributor, maintainer, security reviewer, performance researcher, and release engineer routes explicitly. Landing pages state audience, prerequisites, supported versions, shortest successful path, failure recovery, and canonical deeper source.

## Documentation workflow

1. Write a documentation contract from the accepted issue or release.
2. Classify content as tutorial, how-to, reference, explanation, troubleshooting, decision, research, or release record.
3. Place it in the canonical surface and add audience navigation.
4. Add exact commands, diagrams, examples, and failure cases where they reduce ambiguity.
5. Validate links, examples, navigation, generated Wiki output, and release references.
6. Record supersession instead of silently rewriting historical decisions or research.

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

## Output

Produce the authority matrix, audience map, navigation tree, proposed files, migration or redirect plan, validation commands, ownership and review cadence, and unresolved decisions. Distinguish fact from recommendation.
