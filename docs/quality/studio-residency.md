# Alpine Studio residency protocol

This protocol captures process memory for Task
[#241](https://github.com/dbuddha/alpine-gpui/issues/241). It complements, but
does not replace, Alpine-owned cache, atlas, upload, queue, and retained-byte
counters. A cache total is not process footprint, and process footprint does
not identify which Alpine owner retained memory.

## Evidence boundary

Physical capture runs only on Apple Silicon macOS. The capture retains Apple's
untouched `footprint` JSON in byte units, the exact workload and environment
records, a derived sample table, a bounded analysis summary, and a manifest
binding the result to a clean full Git revision, exact process executable and
binary hash, derived record hashes, process-start identity, warmup, sampling
interval, and post-close observation. CI checks the parser and rejection rules
with synthetic fixtures; CI does not turn hosted-machine samples into
performance evidence.

The v1 capture targets one process. Local language servers and other child
processes require separate artifacts and must be reported beside the Studio
process. Omitting them from a product-level total is invalid. GPU allocations,
allocator samples, and Alpine cache counters remain separate evidence inputs;
this script does not infer them from resident memory.

## Capture

Prepare versioned workload and environment records, hash them, launch the exact
release binary, perform the declared journey, and close Studio through its
production UI during the post-capture timeout. A four-hour capture at a
15-second interval produces fewer than 1,000 samples and remains below the
4,096-sample artifact bound.

```sh
scripts/capture-studio-residency.sh \
  --pid "$STUDIO_PID" \
  --binary target/release/alpine-studio \
  --repository . \
  --output-dir target/residency/alpine-repository-long-edit \
  --revision "$(git rev-parse HEAD)" \
  --workload target/residency-inputs/alpine-long-edit-workload.toml \
  --environment target/residency-inputs/apple-silicon-environment.toml \
  --duration-seconds 14400 \
  --interval-seconds 15 \
  --warmup-seconds 600 \
  --post-close-timeout-seconds 60
```

The capture copies both input records into its immutable output and derives
their SHA-256 identities itself. Caller-supplied hashes are not accepted. The
declared repository must be clean at the declared revision, and the target PID
must expose the exact canonical executable path supplied by `--binary`.

The first captures are informational baselines. A blocking window adds
`--slope-limit-bytes-per-second` only after A/A calibration has established a
revisioned threshold. The analyzer computes ordinary least-squares slopes over
the post-warmup window for physical footprint and private dirty bytes. A window
passes only when both slopes are at or below the supplied bound. One passing
window does not prove bounded residency; repeated independent windows, cache
churn, pressure recovery, and defect-free post-close observations are required.

## Required workload families

- Warm typing and scrolling over an unchanged code viewport.
- File switching, quick open, bounded project search, and search-result churn.
- Diagnostics, completion, navigation, cancellation, and language-server
  restart with the server measured separately.
- Hide, show, minimize, restore, sleep, wake, and display migration.
- Large-file and large-workspace cache pressure followed by an idle recovery
  window.
- Production close with no surviving Studio process and no unexplained worker.

Each workload states included behavior, file set, duration, warmup, expected
cache phase, and exclusions. A positive slope is a defect candidate, not proof
of a leak by itself. Investigation must distinguish intended bounded cache
admission, allocator retention, GPU residency, language-server memory, and
unreachable ownership before changing a budget.

## Fair comparison

Alpine and pinned Zed runs use the same physical machine, OS build, display,
power, thermal window, repository snapshot, font set, language server, warmup,
journey, and sampling protocol. AB/BA order is randomized. Studio-only and
language-server footprints are reported separately, then combined only for a
matched product journey. Alpine-owned byte counters are never compared with
Zed's process footprint. Dominance remains unavailable until semantic, input,
IME, accessibility, lifecycle, and rendered-output equivalence are green and
the comparator protocol's independent-window statistics are satisfied.

## CI contract

`scripts/test-studio-residency.sh` proves that stable samples pass a calibrated
window, positive growth fails, process identity cannot drift, timestamps are
strictly monotonic, sampled peaks are valid, non-byte evidence is rejected, and
warm-window bounds are enforced. It does not prove macOS measurement accuracy,
long-session boundedness, post-close baseline recovery, or comparative
superiority. Those require retained physical-hardware artifacts under #241.

## Rejected AX process captures

`scripts/capture-studio-ax-process.sh` combines raw AX-client output with process
residency input for Task [#479](https://github.com/dbuddha/alpine-gpui/issues/479).
Its successful output remains an intermediate Task #504 package, not accepted
[physical accessibility evidence](../aep/0273-physical-accessibility-qualification.md).

On failure, the wrapper reports a separate private `.alpine-ax-rejected.*`
directory beside the requested output. It never publishes a successful manifest
there or overwrites an existing destination. `rejection.txt` records the failure
phase, exit statuses, available source and executable identities, and an explicit
unqualified disposition. `artifacts.tsv` records unavailable or unsafe paths,
observed source sizes, retained prefix sizes, SHA-256 hashes of those prefixes,
and truncation. Missing evidence is not recorded as a zero measurement.

Each allowlisted artifact is capped at 64 KiB, diagnostic metadata at 16 KiB,
and total retained logical file bytes at 1 MiB. These are file-content limits,
not a physical-footprint or filesystem-allocation claim. Symlinked files and
artifact parent directories are not copied. Directory permissions are 0700;
retained files are private to the capturing user. Raw diagnostics can contain
workspace text and must not be uploaded automatically.

The wrapper retains the existing capture-owned child termination policy. A
rejected prefix is not an atomic multi-file snapshot, proof of producer
quiescence, normal process exit, accessibility trust, or a qualified journey.
If diagnostic retention fails, the original temporary capture remains in place
and its path is reported; that fallback is not a bounded published package.
Review and retain useful evidence before explicitly removing such directories.

The fixture suite covers partial raw output followed by failure, denied-client
simulation, missing output, sampler and child errors, close timeout, truncation,
hash integrity, private permissions, unsafe paths, destination collisions, and
failed diagnostic writes. Defect [#580](https://github.com/dbuddha/alpine-gpui/issues/580)
owns this correction; these controls do not close physical M4 or M5 acceptance.
