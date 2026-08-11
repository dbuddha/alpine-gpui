# Contributing

Alpine GPUI is currently a private, single-maintainer project operated through
short-lived branches and protected pull requests.

## Before implementation

1. Read `AGENTS.md` and the nearest scoped `AGENTS.md`.
2. Read `docs/README.md`, `ARCHITECTURE.md`, and `docs/MASTER_PLAN.md`.
3. Inspect the current branch, remote tracking state, and dirty files.
4. State the objective, excluded scope, acceptance gate, and owner decisions.
5. Create a short-lived branch for one coherent change.

## Change requirements

Every change must:

1. preserve documented ownership boundaries;
2. include tests proportional to correctness, lifetime, security, and
   performance risk;
3. pass `scripts/check.sh`;
4. update documentation or an ADR when behavior or architecture changes;
5. record upstream influence or incorporated source in the appropriate
   research and provenance artifacts;
6. avoid dependency changes without explicit approval;
7. include a change fragment when users, public APIs, compatibility,
   performance, or security are affected.

## Commit and PR format

Commit subjects and PR titles use:

```text
type(scope): imperative summary
```

Examples:

```text
feat(scene): add hierarchical clip primitives
fix(runtime): coalesce duplicate frame requests
perf(metal): reuse transient upload buffers
docs(governance): define source influence policy
```

Use the pull request template. Evidence belongs in the PR and repository, not
only in chat. Branches are squash-merged so `main` receives one logical commit
per PR.

## Performance evidence

Performance changes must include the benchmark definition and raw results.
Hardware-specific results must record machine, OS, toolchain, display, power
state, warmup, sample count, distribution, and variance.

## External actions

Owner approval is required before pushing, opening a PR, changing dependencies,
creating a release, publishing an artifact, or changing paid runner settings.
