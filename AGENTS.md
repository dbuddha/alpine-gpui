---
schema: alpine-agent-policy/v1
scope: repository
---

# Alpine GPUI Change Policy

## Scope

This is the complete repository operating contract. Do not add nested agent
files. `README.md` owns stable purpose, `ARCHITECTURE.md` owns implemented
truth, the engineering guide owns durable product knowledge, rustdoc owns APIs,
the evidence registry owns claim mappings, and GitHub owns live capabilities,
requirements, tasks, decisions, research, results, and releases.

## Required context

Before changing anything:

1. Identify repository, branch, upstream, and dirty state.
2. Read this file completely.
3. Read the linked task or requirement and its parent chain, stopping at
   Capability -> Requirement -> Task.
4. Confirm the capability and requirement have `owner:approved`.
5. Read `ARCHITECTURE.md` for subsystem, ownership, unsafe, renderer, platform,
   or architecture work.
6. Read the linked AEP and registry claims for consequential design or formal
   work. Read only case studies, decisions, or research linked by that chain.
7. Fetch `origin` before comparing the branch with its base.

If observable acceptance or required approval is missing, stop and ask. Do not
turn chat into a durable requirement without an approved GitHub issue.

## Safety and dependencies

- Preserve unrelated and uncommitted user work.
- Never force push, rewrite published history, bypass a gate, or hide a failure.
- Never commit secrets, private data, build output, or machine configuration.
- Safe Rust is the default. Unsafe code requires owner approval, a local safety
  argument, focused tests, adversarial review, and `review:unsafe`.
- Native handles, callbacks, queues, surfaces, and teardown are explicit
  ownership boundaries.
- Do not hide allocation, upload, synchronization, or continuous redraw.
- Ask before adding, removing, or materially changing any dependency.
- Shipping manifests must not use Git dependencies.
- Admit dependencies only behind Alpine-owned boundaries after license,
  maintenance, features, transitive graph, lifecycle, and performance review.
- Do not copy upstream source without owner approval. Source adaptation requires
  `review:provenance`, `provenance.toml`, notices when required, and independent
  tests in the same pull request.

## Branches, commits, and pull requests

- Use a focused branch and normally one task per pull request.
- Keep behavior and its tests in the same logical commit.
- Use `type(scope): summary` Conventional Commit messages with no WIP or agent
  attribution. Commits must remain buildable and bisectable.
- Inspect status, all diffs, untracked files, and recent history before commit.
- Ask the owner immediately before every push and release publication.
- Every implementation PR closes or contributes to a requirement or task and
  links its parent capability.
- Every PR has exactly one `release:breaking`, `release:feature`, `release:fix`,
  or `release:none` label.
- Architecture changes link an accepted decision. Upstream influence links the
  exact research record and states the influence mode.
- Consequential behavior lists exact AEP claim and evidence IDs in the PR.
- Complete every PR template section with concrete evidence and remaining risk.
- Squash merge only after required checks and conversations are resolved.

## Owner approval required

Get explicit approval before changing a public contract, supported platform,
architecture boundary, performance budget, required gate, CI provider, unsafe
code, dependency, license, source adaptation, repository setting, secret, or
release. Approval must be visible on the relevant issue or pull request.
`owner:approved` represents approval only for a capability or requirement.

## Engineering and evidence

- Define acceptance evidence before implementation.
- Prefer validated domain types, private representation, meaningful errors,
  explicit ownership, common traits, `must_use`, and documented failure modes.
- Add the narrowest proof: unit or property tests for pure behavior, model or
  integration tests for boundaries, and native validation for backend behavior.
- TLA+ models declare finite bounds, fairness, exclusions, mapped Rust events,
  and a faulty configuration that must expose a known invariant violation.
- Kani harnesses live in crate-local `cfg(kani)` modules, use stable explicit
  proofs and cover statements, disclose assumptions, and link dynamic tests.
- Report model checking separately from implementation verification. Never
  claim refinement unless an accepted decision introduces a verified proof.
- Rendering changes require semantic or CPU oracles and offscreen readback
  before image comparison. A cross-GPU pixel hash is not a sufficient oracle.
- Concurrency, unsafe, parser, lifecycle, and hot-path changes add Kani, Miri,
  Loom, fuzzing, mutation, coverage, or fixed-hardware evidence by risk.
- Kani proves selected bounded Rust properties. Lean requires a separate,
  owner-approved research decision and is not a default dependency or gate.
- Performance claims require distributions and qualified hardware. Hosted Mac
  timing is informational until a fixed machine is qualified.
- Never rerun, weaken, or disable a failing check to obtain green CI. A flaky
  test is a defect and must be tracked.

Run before commit and again before requesting a push:

```sh
scripts/check.sh
```

Hosted `ci-pass` is authoritative. Local success is supporting evidence.

## Definition of done

A change is done only when scope matches the approved parent chain, acceptance
tests cover success and failure, risk-selected evidence passes, architecture and
rustdoc are current, dependency and provenance policy is satisfied, the PR
records exact results and remaining risk, `ci-pass` succeeds, and the merged PR
closes its task. Requirements and capabilities close only when all child work
and end-to-end acceptance are complete.
