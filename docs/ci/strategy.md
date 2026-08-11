# CI and Hardware Strategy

## Principles

- CI is authoritative; local checks are a fast preview.
- Cheap deterministic work runs before scarce Apple GPU work.
- Every workflow and test command lives in the repository.
- CI actions use immutable commit SHAs.
- The committed lockfile is tested with `--locked`; dependency updates are
  isolated, reviewed changes.
- Hardware capability is proven by a checked-in probe, not provider marketing.
- Correctness and performance are separate gates.

## Day-one workflows

The initial `CI` workflow has two layers:

1. Linux policy gate: format, Clippy, tests, and documentation.
2. Native matrix: locked workspace tests on Ubuntu, Apple Silicon macOS, and
   Windows.

A stable `ci-pass` aggregation job is the future branch-protection target.
No cache action is used initially. This reduces supply-chain surface and makes
the baseline cost and reproducibility visible before optimization.

## What is available without sourcing hardware

The current personal GitHub Pro account can use GitHub's standard
`macos-26` runner directly. GitHub documents that label as a three-core M1
machine with 7 GB of memory for private repositories. The workflow uses this
runner for Apple Silicon compilation and platform-independent tests from the
first push.

GitHub Pro currently includes 3,000 standard-runner minutes per month. Beyond
included usage, the standard macOS rate is currently $0.062 per minute. These
facts should be rechecked before changing the budget:

- [GitHub-hosted runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
- [GitHub Actions included usage](https://docs.github.com/en/billing/concepts/product-billing/github-actions)
- [GitHub Actions runner pricing](https://docs.github.com/en/billing/reference/actions-runner-pricing)

This is sufficient for ordinary CI and is the correct day-one choice. The Metal
probe must still establish whether its virtualization exposes every feature
needed for offscreen validation. It is not a trustworthy performance baseline.

## Metal runner qualification

The future `alpine-metal-probe` binary and workflow must record:

- OS build, Xcode, Rust, CPU, memory, and runner provider;
- Metal device name, registry ID, unified-memory status, GPU families, limits,
  and feature queries;
- queue, buffer, texture, pipeline, command-buffer, readback, and teardown
  behavior;
- validation failures and device-loss behavior;
- counter and timestamp availability;
- cold and warm timing distributions;
- whether WindowServer and visible presentation are available.

The qualification job runs manually first. A provider becomes a required gate
only after its capability report is stable and the repository-specific access
review passes.

## Recommended provisioning

### Current personal repository

1. Use standard GitHub runners for the initial matrix.
2. Trial a Depot M4 runner for offscreen Metal correctness after M1 exists.
3. Install the provider GitHub App for this repository only.
4. Use ephemeral runners, a $50 provider budget, and no signing secrets.
5. Promote the runner only after the checked-in probe passes.

Depot currently advertises an eight-core M4 macOS 26 runner with 24 GB of
memory at $0.08 per minute. It can be installed for only this repository, so it
is the preferred paid experiment if the standard runner lacks a required Metal
capability. Capacity is not fully elastic, so it is not yet a sole required
gate. See [Depot runner types](https://depot.dev/docs/github-actions/runner-types).

### If moved to a GitHub organization

Prefer GitHub's ARM64 macOS XLarge runner for proprietary source isolation and
explicit GPU acceleration. GitHub currently documents this as a five-core M2
with eight GPU cores at $0.102 per minute. It requires GitHub Team or Enterprise
and is billed outside the included allowance.

### Fixed performance hardware

Ephemeral cloud machines can gate Metal correctness, but not stable performance.
A pinned physical Mac is required before performance regressions block merges.
The first fixed machine should represent the oldest supported Apple Silicon
family. It may be owned, rented as a dedicated host, or scheduled periodically.

## Budget policy

- Initial operating target: $60 per month.
- Hard stop: $75 per month until explicitly raised.
- Use path filters and an orchestration job before paid Metal work.
- Upload artifacts only on failure, with short retention.
- Consolidate expensive GPU checks into one job per revision.
- Run deep fuzz and performance work manually until signal and cost are known.

## Planned gates

| Gate | PR | Main | Nightly | Fixed hardware |
| --- | --- | --- | --- | --- |
| Format, Clippy, docs | Required | Required | Yes | No |
| Unit and property tests | Required | Required | Extended | No |
| Three-OS compile/test | Required | Required | Yes | No |
| Metal capability probe | Relevant changes | Required | Yes | No |
| Offscreen Metal goldens | Relevant changes | Required | Extended | No |
| Sanitizers and Miri subset | No | No | Yes | No |
| Fuzz corpus | No | No | Yes | No |
| Frame-time regression | No | Informational | Required later | Yes |
| Display and input latency | No | No | Release | Yes |

## Repository settings

- `main` is the default protected branch.
- Pull requests and a strict, up-to-date `ci-pass` are required.
- Linear history and conversation resolution are required.
- Force pushes, branch deletion, and administrator bypass are disabled.
- Actions are restricted to GitHub-owned actions and full-length commit SHAs.
- The workflow token is read-only by default.
- Configure an Actions budget with stop-on-limit enabled.
- Keep artifact retention short.
