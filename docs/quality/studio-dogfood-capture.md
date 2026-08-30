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
`alpine-studio-dogfood/v1` bundles. Studio can emit one bounded
`alpine-studio-internal-diagnostic/v1` JSON record after a clean close when all
four explicit local environment values are present. The output copies final
runtime, surface, queue, cache, language, accessibility, and lifecycle evidence,
refuses overwrite and symbolic-link traversal, and identifies unavailable
stage timing, process sampling, and cache axes as omissions. It performs no
default-path I/O when the contract is absent.

The live capture path added for
[Task #462](https://github.com/dbuddha/alpine-gpui/issues/462) launches the exact
Studio executable, samples its process with macOS `footprint`, retains the raw
internal and process records, and atomically seals an omission-aware version 2
bundle. Version 1 fixtures remain immutable protocol controls. A version 2
fixture is explicitly marked `fixture`; it is not physical dogfood evidence and
establishes no baseline or comparative claim.

The opt-in internal contract requires:

- `ALPINE_STUDIO_DOGFOOD_OUTPUT`: normalized absolute new JSON file path.
- `ALPINE_STUDIO_DOGFOOD_WORKLOAD_ID`: bounded lowercase workload slug.
- `ALPINE_STUDIO_DOGFOOD_REVISION`: exact lowercase 40-character revision.
- `ALPINE_STUDIO_DOGFOOD_CAPTURED_AT_UTC`: UTC capture start timestamp.

Partial identity, an existing output, an unavailable parent, and a parent path
that traverses a symbolic link fail before publication. Task #462 owns the only
supported operator-facing launcher for this contract.

## Bundle contracts

Version 1 contains the original `session.toml` and `snapshot.toml` controls.
Version 2 additionally retains `internal-diagnostic.json`, `footprint.json`,
`studio.stdout`, and `studio.stderr`. Its manifest hashes every retained file
other than the self-referential manifest, plus the exact Studio executable and
sampler. Validation reparses both raw JSON records and requires the normalized
snapshot to reproduce them exactly.

Version 2 never substitutes one evidence class for another. Physical footprint
and private dirty bytes come only from Apple `footprint`. Alpine cache and queue
bytes come only from the internal diagnostic. Process GPU bytes and per-sample
Alpine-owned bytes remain absent fields with explicit omissions until a trusted
sampler exists. They are never encoded as zero.

## Version 1 bundle contract

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

## Capture a live version 2 bundle

Prepare a version 2 draft containing the workload, workspace fixture, settings,
font, language-server, and environment identities. Build the exact release app,
then run the explicit local capture action. The output parent must already exist
and the destination must not.

```sh
scripts/capture-studio-dogfood.sh \
  --binary 'target/release/Alpine Studio.app/Contents/MacOS/alpine-studio' \
  --repository . \
  --workspace . \
  --draft path/to/draft-v2.toml \
  --output-dir target/dogfood/alpine-repository-session \
  --workload-id alpine-repository-edit \
  --duration-seconds 300 \
  --interval-seconds 5 \
  --post-close-timeout-seconds 60 \
  --opt-in
```

Normal capture requires Apple Silicon macOS and the system
`/usr/bin/footprint`. The script checks a clean exact repository revision,
the release bundle's embedded revision and executable hash, canonical process
identity, process-start identity before and after sampling, bounded duration and
interval values, successful process exit, final internal output, and a new
destination. The sealer independently requires the internal revision and capture
timestamp to match the launcher's values. It performs no network operation and
uploads nothing. Closing Studio is an explicit human action after sampling.

`--fixture-only --sampler PATH` exists solely for the headless fake-process and
fake-sampler regression. Its manifest records `evidence_scope = "fixture"`, so
it cannot qualify a physical result.

## Pass, failure, and qualification semantics

A failed Studio session may be sealed so defects are not lost. Its outcome must
be `failed`, and its local status should identify the linked defect. A passed
session additionally requires zero idle submissions, one completed close, clean
shutdown, and post-close bytes within the declared bound.

Validation proves schema integrity, identity binding, bounded retained data,
raw-to-normalized reproduction, checksums, and selected internal invariants. It
does not prove that a fixture used a physical machine, that operating-system
measurements are accurate, or that a workload is semantically equivalent to
Zed. Tasks #240 and #241 still own physical-hardware calibration,
distributions, slope analysis, and invalidation rules. Zed and GPUI claims remain
governed by the separate comparator protocol.

## Validation gate

`scripts/test-dogfood-capture.sh` validates the fixture and report, records and
revalidates a sealed bundle, refuses overwrite, detects snapshot tampering, and
rejects telemetry-enabled input. It runs in `scripts/check.sh` on every change.

`scripts/test-live-dogfood-capture.sh` runs a headless fake Studio process and
fake sampler through the complete version 2 launcher, sealer, validator, and
reporter. It proves checksum tamper rejection, overwrite refusal, explicit GPU
omission, fixed retained-file inventory, malformed sampler rejection, missing
internal-output rejection, nonzero process-exit rejection, and revision-drift
rejection without opening a native window.
