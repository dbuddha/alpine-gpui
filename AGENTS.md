# Alpine GPUI Operating Contract

This file is the repository-wide instruction source for coding agents and human
contributors. More specific `AGENTS.md` files refine these rules for their
directory. The nearest applicable file wins only where it is more specific; it
does not relax repository safety, provenance, or verification requirements.

## Mission

Build a proprietary, Rust-first desktop application framework that owns its
runtime, scheduling, scene protocol, renderer policy, resource lifetimes, and
native platform integrations. Apple Silicon on macOS 15 or newer is the
flagship target. Direct Vulkan on Linux and direct D3D12 on Windows follow once
the Metal and shared semantic contracts are proven.

Alpine GPUI is optimized first for editors, terminals, database tools, and
data-heavy productivity applications. Web, mobile, and Intel Macs are outside
the version 1 scope.

## Accepted product decisions

- Use familiar GPUI concepts without source compatibility.
- Deliver essential components through measured vertical slices.
- Use typed Rust styling and theme tokens, with no CSS runtime.
- Keep layout behind an Alpine facade. Taffy may be an oracle or temporary
  provider, not an unquestioned permanent dependency.
- Use CoreText first behind a portable Alpine text contract.
- Make embedded Metal surfaces and custom materials first-class capabilities.
- Share semantics across platforms while adapting appearance and behavior.
- Permit audited native bindings and standards-heavy Rust libraries behind
  Alpine-owned facades.
- Permit only permissive shipping licenses such as Apache-2.0, MIT, BSD, ISC,
  and Zlib unless the owner approves an exception.
- Treat accessibility as part of every interactive component from its first
  implementation.
- Dogfood with Alpine Lab and a data-heavy Alpine Workspace.
- Define aggressive provisional performance budgets, then calibrate them on
  fixed M1-class hardware.

## Required reading

Before non-trivial work, read in this order:

1. `docs/README.md`
2. `ARCHITECTURE.md`
3. `docs/MASTER_PLAN.md`
4. `docs/ROADMAP.md`
5. The nearest scoped `AGENTS.md`
6. Relevant ADRs, research notes, dependency decisions, and CI documentation

Do not rely on chat history when a repository artifact contains the decision.
If code and documentation conflict, stop and report the conflict.

## Repository map

| Path | Responsibility |
| --- | --- |
| `crates/` | Shipping Rust crates and their tests |
| `docs/adr/` | Durable architectural decisions |
| `docs/research/` | Upstream evidence, source map, and provenance |
| `docs/engineering/` | Contribution, release, and agentic workflow |
| `docs/ci/` | CI, runner, and performance-gate policy |
| `changes/` | Per-PR user-facing change fragments |
| `.github/` | Pull request templates and pinned CI workflows |
| `scripts/` | Repository-owned deterministic gates |

## Architectural boundaries

- Keep scene types independent of windowing and graphics APIs.
- Keep platform policy out of renderer contracts.
- Keep native GPU handles out of application, view, and scene state.
- A renderer may retain backend resources, but never application or view
  objects.
- Portable semantics live above the scene boundary. Backend capability and
  specialization live below it.
- Do not force Metal through a least-common-denominator GPU abstraction.
- Rendering is demand-driven. A settled application does not redraw merely
  because an event loop or display clock ticks.
- Scene construction, resource preparation, encoding, submission, and present
  must remain separately observable.
- Device loss, allocation failure, unsupported capabilities, and platform
  failures are structured errors, not process panics.

## Source and provenance rules

GPUI, GPUI-CE, `gpui-component`, WGPUI, the `gpui-wgpu` lineage, Kael, and
other projects are specimens, not product dependencies.

- Prefer a clean implementation informed by public architecture and behavior.
- Record every researched repository at an immutable commit in
  `docs/research/source-map.md`.
- Record behavioral lessons and test cases in the relevant research note.
- Do not copy source merely because its license permits copying.
- Before incorporating source, obtain owner approval and add a provenance
  ledger entry naming the destination symbol, exact source URL and commit,
  license, modifications, tests, reviewer, and date.
- Do not copy names, trademarks, private assets, or product-specific behavior.
- Do not introduce GPUI, WGPU, winit, Blade, or a GPUI fork as a production
  dependency without a new ADR and explicit owner approval.

## Dependency and unsafe-code rules

- Add or remove no dependency without owner approval.
- Production manifests may not use Git dependencies.
- Disable default features unless each enabled feature is intentional.
- Review license, maintenance, transitive graph, unsafe surface, runtime policy,
  allocator behavior, and replacement strategy before admission.
- Keep `unsafe` out of safe crates. Native FFI belongs in narrowly scoped crates.
- Every unsafe block must state its safety invariant immediately above it.
- Safe wrappers must prevent invalid lifetime, thread, ownership, and aliasing
  states rather than merely documenting them.

## Rust implementation rules

- Use the pinned toolchain and committed lockfile.
- Treat all warnings as errors.
- Do not use `unwrap`, `expect`, `panic`, `todo`, or `dbg` in shipping paths.
- Prefer explicit domain errors over strings.
- Avoid hidden allocation, cloning, reference counting, dynamic dispatch, and
  synchronization in hot paths.
- Use newtypes for units, coordinate spaces, resource identities, and revisions.
- Make ownership and lifetime transitions visible in APIs.
- Keep public APIs minimal until a vertical slice proves their shape.
- Do not add abstraction without at least two concrete consumers or a documented
  boundary requirement.

## Performance evidence

- Make no performance claim without a reproducible benchmark and raw results.
- Record hardware, OS, toolchain, display, power state, sample count, warmup,
  distribution, and variance.
- Compare distributions on pinned hardware. Ephemeral CI is for correctness and
  smoke measurements, not merge-blocking microbenchmarks.
- Instrument allocations, uploads, draw calls, command buffers, submitted
  frames, and cache growth before optimizing them.
- Performance fixes must include a regression test or benchmark that fails on
  the prior behavior.

## Testing contract

Define the acceptance gate before implementation. Use the smallest useful test
first, then run the repository gate before handoff or commit.

Required baseline:

```sh
scripts/check.sh
```

Add risk-proportional evidence:

- Geometry and state machines: unit and property tests.
- Scheduling and lifetimes: deterministic model tests and adversarial ordering.
- Renderer work: semantic scene snapshots, CPU oracles, offscreen readback, and
  GPU validation.
- Components: keyboard, pointer, focus, accessibility, scale, theme, and visual
  fixtures.
- Unsafe or FFI changes: boundary tests, negative tests, and platform validation.
- Performance changes: before-and-after benchmark distributions.

A flaky test is a defect. Do not retry it into green or mark a security,
validation, or correctness gate non-blocking without an owner and expiry.

## Branch, commit, and pull request workflow

- Never work directly on protected `main` after the initial foundation.
- Use a short-lived branch containing one coherent change.
- Before editing, inspect branch, status, remote tracking, and relevant history.
- Before pushing, fetch `origin` and compare the branch with `origin/main`.
- Ask before every push, pull request, release, or external publication.
- Use Conventional Commit form for commit subjects and PR titles:
  `type(scope): imperative summary`.
- Allowed types are `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `build`,
  `ci`, `chore`, and `revert`.
- Keep commits independently understandable. Temporary fixup commits may exist
  on a branch, but the PR is squash-merged as one logical main-branch commit.
- Never include AI attribution in commits, source, changelogs, or PR text.
- Complete every applicable section of the pull request template.
- Add a change fragment for user-visible behavior, performance, security, API,
  or compatibility changes. Documentation-only and internal test changes may
  state `Not applicable` in the PR.

## Durable record selection

Put information in the narrowest artifact future contributors will load:

- Product goal or invariant: `ARCHITECTURE.md` or `docs/MASTER_PLAN.md`
- Architectural choice: ADR
- Upstream observation: research note
- Copied or adapted source: provenance ledger
- Dependency choice: `docs/DEPENDENCIES.md` and an ADR when architectural
- User-visible change: `changes/` fragment, then `CHANGELOG.md` at release
- Workflow correction: relevant `AGENTS.md` or engineering runbook
- Benchmark fact: checked-in benchmark definition and results artifact policy

## Human-owned decisions

Stop and ask before:

- adding or removing dependencies;
- expanding supported platforms or version 1 scope;
- accepting a non-permissive license;
- copying upstream source;
- changing public API compatibility policy;
- installing a runner provider or changing paid CI budgets;
- introducing signing or notarization credentials;
- pushing, creating a PR, releasing, publishing, or distributing artifacts;
- weakening a test, security, provenance, or performance gate.

## Definition of done

A change is done only when:

1. scope and acceptance gates are explicit;
2. implementation respects ownership boundaries;
3. tests cover the relevant failure modes;
4. documentation, ADRs, provenance, and change fragments are updated as needed;
5. `scripts/check.sh` passes;
6. status, staged and unstaged diffs, and untracked files are reviewed;
7. the final report names evidence, limitations, and work not performed.
