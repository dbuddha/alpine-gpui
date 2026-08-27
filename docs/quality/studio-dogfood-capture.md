# Alpine Studio dogfood capture

This protocol records local Alpine Studio sessions as revision-bound evidence for
[Task #238](https://github.com/dbuddha/alpine-gpui/issues/238). It supports the
later startup and interaction baselines in
[Task #240](https://github.com/dbuddha/alpine-gpui/issues/240) and residency
qualification in
[Task #241](https://github.com/dbuddha/alpine-gpui/issues/241).

The repository page is canonical. The GitHub Wiki is a revision-pinned retrieval
mirror, Issues own live status, and a validated bundle owns facts about one
captured session.

## Current implementation status

`alpine-assurance` can validate, report, and atomically seal
`alpine-studio-dogfood/v1` bundles. It does not yet launch Studio, sample a live
process, or ask Studio to emit its internal diagnostic snapshot. Those are
remaining Task #238 implementation slices. The committed fixture proves the
protocol and its rejection controls only; it is not physical dogfood evidence
and establishes no baseline or comparative claim.

## Bundle contract

A bundle contains exactly these canonical inputs:

- `session.toml` identifies the Alpine revision, workload, duration, local
  workspace fixture, settings, font, language server, environment, coverage,
  assumptions, exclusions, and snapshot checksum.
- `snapshot.toml` records bounded internal and process evidence without document
  contents, file paths, keystrokes, source text, or network identifiers.

The manifest binds the snapshot by SHA-256. Unknown fields, malformed values,
absolute or escaping snapshot paths, files above the protocol ceiling, and
checksum drift fail validation.

Every manifest must assert explicit opt-in, no telemetry, no network I/O, and no
performance claim. It must list `telemetry`, `network-io`, and
`comparative-claim` as exclusions. A real hardware identifier should be a stable
pseudonym rather than a serial number or user identity.

## Required evidence classes

Frame evidence preserves requested, submitted, completed, presented, omitted,
idle, and peak in-flight counts. A passed capture admits at most three in-flight
frames and no idle submission.

Timing remains separated into these stages:

- `scene-build`
- `adaptation`
- `atlas-upload`
- `encode`
- `commit`
- `gpu-completion`
- `presentation`
- `input-to-present`
- `shutdown-drain`

The capture must not place parsing or adaptation inside one renderer's interval
and outside another renderer's interval. These local records do not become
comparative evidence until the comparator protocol admits an equivalent pinned
workload and qualified environment.

Every snapshot records current, peak, and budget bytes for:

- `layout-cache`
- `syntax-cache`
- `glyph-atlas-cpu`
- `glyph-atlas-gpu`
- `font-cache`
- `fallback-cache`
- `language-process`
- `foreground-queue`
- `background-queue`
- `upload-staging`

Process samples retain sequence, elapsed time, physical footprint, private dirty
memory, GPU bytes, and Alpine-owned retained bytes. A capture retains at most
4,096 process samples and one MiB of snapshot TOML. Current and peak language
payloads, accessibility nodes, stale work, restart counts, close completion,
clean shutdown, and post-close bytes remain explicit rather than inferred from
process exit.

## Seal a local bundle

Start from a draft manifest with the same fields as
`assurance/dogfood/v1/session.toml`. The recorder replaces only
`snapshot_file` and `snapshot_sha256`; all other identities must already describe
the actual session.

```sh
cargo run --quiet --locked -p alpine-assurance -- \
  record-studio-dogfood \
  path/to/draft-session.toml \
  path/to/snapshot.toml \
  path/to/new-bundle
```

The destination parent must exist. The destination and its sibling staging path
must not exist. The recorder validates both inputs, writes a hidden sibling
staging directory, validates the staged bundle through the normal CLI boundary,
and renames it into place. It never overwrites an existing destination. An
ordinary failure removes its own staging directory; after process or machine
interruption, inspect any retained staging directory before choosing a new
destination.

Validate and render a bounded report with:

```sh
cargo run --quiet --locked -p alpine-assurance -- \
  validate-studio-dogfood path/to/bundle/session.toml

cargo run --quiet --locked -p alpine-assurance -- \
  studio-dogfood-report path/to/bundle/session.toml
```

The report is descriptive. It does not calculate percentiles, confidence
intervals, slopes, equivalence, or dominance.

## Pass, failure, and qualification semantics

A failed Studio session may be sealed so defects are not lost. Its outcome must
be `failed`, and its local status should identify the linked defect. A passed
session additionally requires zero idle submissions, one completed close, clean
shutdown, and post-close bytes within the declared bound.

Validation proves schema integrity, identity binding, bounded retained data, and
selected internal invariants. It does not prove that values came from an
unmodified Studio binary, that operating-system measurements are accurate, or
that a workload is semantically equivalent to Zed. Task #238 must add automatic
revision-pinned Studio emission and process sampling before a bundle is accepted
as real dogfood evidence. Tasks #240 and #241 then add physical-hardware
calibration, distributions, slope analysis, and invalidation rules. Zed and GPUI
claims remain governed by the separate comparator protocol.

## Validation gate

`scripts/test-dogfood-capture.sh` validates the fixture and report, records and
revalidates a sealed bundle, refuses overwrite, detects snapshot tampering, and
rejects telemetry-enabled input. It runs in `scripts/check.sh` on every change.
