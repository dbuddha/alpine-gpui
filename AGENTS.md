---
schema: alpine-agent-policy/v1
scope: repository
---

# Alpine GPUI Change Policy

## Scope

This file is the complete repository operating contract for humans and agents.
Do not create nested `AGENTS.md` files. Keep stable project facts in `README.md`
and implemented technical truth in `ARCHITECTURE.md`. GitHub owns planning,
decisions, research, delivery state, and shipped history.

## Required context

Before changing anything:

1. Identify the repository, branch, upstream, and dirty state.
2. Read this file completely.
3. Read the linked GitHub issue and follow its parent links, stopping after the
   journey, requirement, and task levels.
4. Read `ARCHITECTURE.md` for subsystem, contract, ownership, unsafe, renderer,
   platform, or architecture work.
5. Read only decision or research issues explicitly linked from the task.
6. Fetch `origin` before comparing the branch with its base.

If no issue defines observable acceptance, stop and ask the owner to clarify or
authorize an issue. Do not infer a durable product requirement from chat alone.

## Safety and ownership

- Preserve user changes and unrelated dirty files.
- Never force push, delete branches, rewrite published history, or bypass a
  required check.
- Never commit secrets, credentials, tokens, private data, generated build
  output, or machine-specific configuration.
- Keep safe Rust as the default. Unsafe code requires owner approval, a written
  safety contract, focused tests, and the `review:unsafe` label.
- Treat native handles, callbacks, queues, surfaces, and resource teardown as
  explicit ownership boundaries.
- Do not hide allocation, upload, synchronization, or continuous redraw in a
  convenience abstraction.

## Dependencies and external influence

- Ask the owner before adding, removing, or materially reconfiguring a
  dependency.
- Shipping Cargo manifests must not use Git dependencies.
- Prefer the standard library and existing workspace code. Admit a crate only
  behind a narrow Alpine-owned boundary after license, maintenance, feature,
  transitive dependency, lifecycle, and performance review.
- Architecture and observable behavior may be studied from linked research.
  Do not copy or adapt upstream source without explicit owner approval.
- A source-level adaptation must add `review:provenance`, `provenance.toml`, and
  any required `THIRD_PARTY_NOTICES.md` in the same pull request. Record the
  destination symbol, immutable source URL and commit, license, modifications,
  reviewer, and independent tests.
- Do not create empty provenance or notice placeholders.

## Branches, commits, and pull requests

- Work on a focused branch and keep one coherent concern per pull request.
- Use Conventional Commit summaries. Keep commit messages concise, with no AI
  attribution.
- Inspect status, staged and unstaged diffs, untracked files, and recent commit
  style before committing.
- Ask the owner immediately before every push and before publishing a release.
- Every implementation pull request closes or contributes to one requirement or
  task issue and links its parent journey when one exists.
- Every pull request has exactly one of `release:breaking`, `release:feature`,
  `release:fix`, or `release:none`.
- Architecture changes link an accepted decision. Upstream-influenced changes
  link the research record and describe the influence mode.
- Complete every pull request template section with concrete evidence. Write
  `None` only when the section genuinely does not apply.
- Squash merge after required checks pass and review conversations are resolved.

## Owner approval required

Get explicit owner approval before:

- pushing, publishing a release, changing repository settings, or granting an
  external service access;
- adding or removing dependencies, unsafe code, source-level external
  adaptation, or license terms;
- changing a public contract, supported platform, architecture boundary,
  performance budget, CI provider, secret, or required merge gate;
- deleting data or files outside the exact scope of an accepted issue.

Approval must be visible in the issue or pull request when it affects future
reviewers. A label applied by the owner may represent approval where policy
defines that meaning.

## Implementation and verification

- Define the acceptance evidence before implementation.
- Add tests at the narrowest useful layer: unit or property tests for pure
  behavior, model or integration tests for boundaries, and native validation
  for backend behavior.
- Rendering changes require offscreen evidence before visual comparison.
- Concurrency, unsafe, parser, and hot-path changes add Miri, Loom, fuzzing,
  mutation, coverage, or fixed-hardware evidence according to risk.
- Never weaken a gate to make a change pass. Fix the cause or document the
  blocker in the issue.
- Run the complete repository gate before commit and again before requesting a
  push:

```sh
scripts/check.sh
```

Hosted `ci-pass` is authoritative. Local success is evidence, not a substitute.

## Definition of done
A change is done only when scope and acceptance match the linked issue, tests
cover new behavior and failures, `scripts/check.sh` passes, architecture truth
is current, dependency and provenance requirements are satisfied, the pull
request records exact evidence and remaining risk, `ci-pass` succeeds, and the
linked issue is closed by the merged pull request.
