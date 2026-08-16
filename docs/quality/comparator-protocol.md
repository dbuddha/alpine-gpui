# Alpine comparator protocol v1

This protocol governs renderer and local-editor comparisons. It prevents a
faster but behaviorally incomplete path, adaptation overhead hidden inside a
renderer result, or one noisy machine run from becoming a performance claim.

## Claim classes

### Renderer-only

Both implementations receive semantically identical prepared scenes. Report
scene adaptation separately, then measure upload, encode, commit, GPU
completion, and presentation as distinct endpoints. The claim applies only to
the listed primitives, dimensions, clip semantics, blend behavior, and target.

### Framework journey

Measure application state mutation, layout, prepaint, paint, upload, encode,
commit, GPU completion, and presentation. Input queueing and background work
are included or explicitly reported as separate stages.

### Product journey

Measure an externally observable editor outcome across Alpine Studio, pinned
Zed stable, and Sublime safe mode. The result is valid only when final bytes,
selection, viewport, visible output, accessibility, lifecycle, and exclusions
match.

### Stock footprint

Report each official product's default footprint separately. Never combine it
with a normalized local-editor comparison or use it to explain a normalized
result.

## Required identities

Every run binds these fields into an environment record and hash:

- repository revision, dirty state, build profile, compiler, linker, and binary
  hash
- comparator release, revision, patch hash, settings hash, and protocol version
- workload schema, workload bytes, workload hash, repository fixture hash, and
  expected semantic result hash
- hardware model, CPU and GPU identity, memory, display, refresh state, scale,
  and window geometry
- OS build, power source, power mode, thermal state, background-process policy,
  and reboot or uptime state
- font files and hashes, grammar versions, language-server binary and hash,
  locale, input source, and environment variables
- cold or warm class, execution order, independent hardware window, run index,
  and invalidation reason when rejected

`workload_identity_hash`, `environment_hash`, and
`exclusion_manifest_hash` are mandatory artifact fields, not prose-only
disclosures.

## Adaptation separation

Comparator trace decoding, validation, normalization, allocation, and native
scene construction are adaptation work. They are measured as a separate
distribution and never included in renderer upload, encode, commit, GPU
completion, or presentation results. A product journey may include adaptation
only when the same externally observable journey requires it for every product,
and the adaptation distribution is still reported independently.

Removing adaptation work from one implementation, prewarming only one side, or
charging protocol conversion to the comparator rather than the tested system
invalidates the paired run.

## Explicit exclusion manifest

Every normalized product run stores a canonical manifest naming disabled and
unavailable features, launch arguments, settings, account state, network
policy, child processes, plugins or packages, background services, and expected
process inventory. Its canonical bytes produce `exclusion_manifest_hash`.

The Alpine manifest must show that collaboration, AI, cloud accounts, remote
development, telemetry, plugins, extension hosts, marketplace, debugger,
terminal, task, and Git UI subsystems are absent. Zed and Sublime manifests
distinguish features disabled by configuration from stock code that remains in
the binary or process. Stock-product footprint is reported separately and can
never be substituted for this normalized comparison.

## Stage boundaries

| Stage | Starts | Ends | Never includes |
| --- | --- | --- | --- |
| Input queue | Native event timestamp | App delegate begins event | State mutation |
| State mutation | Delegate begins event | New revision committed | Background completion |
| Adaptation | Comparator trace accepted | Native scene ready | Renderer upload |
| Scene build | Dirty revision selected | Immutable scene finished | Adaptation |
| Upload | First resource write | Last required write encoded | GPU execution |
| Encode | First command encoding | Command buffer ready | Queue wait |
| Commit | Commit call begins | Commit returns | GPU completion |
| GPU completion | Commit returns | Terminal command completion | Presentation |
| Presentation | Commit returns | Drawable `presentedTime` observed | Optical scanout |
| Optical | Input stimulus | Qualified photodiode transition | Proxy timestamps |

Apple defines `presentedTime` as the host time when a drawable was displayed and
returns zero when it was not presented or was dropped
([Apple](https://developer.apple.com/documentation/metal/mtldrawable/presentedtime)).
It is presentation evidence, not optical evidence.

## Workload families

| Family | Required cases |
| --- | --- |
| Renderer | Solid quad, clipped quad grid, glyph grid, code viewport, selection and caret, small scroll delta, resize |
| Startup | Empty window, direct file open, restored Alpine repository, first accepted keystroke |
| Editing | Unicode typing, burst typing, undo and redo, multi-cursor, IME, large paste |
| Navigation | Quick open, file switch, split navigation, symbol search, definition |
| Large data | Million-line file, long lines, 100,000-file index, bounded result stream |
| Language | Diagnostic storm, completion burst, stale response, crash and restart |
| Lifecycle | Idle, hide and show, minimize and restore, resize, sleep and wake, interruption, close during work |
| Residency | Cold start, warm steady state, cache churn, long edit and scroll soak, post-close baseline |

## Correctness admission

Before timing, each implementation must pass:

- exact final file bytes and line-ending policy
- selection, cursor, viewport, scroll, and active-document identity
- equivalent visible primitives or an accepted pixel tolerance backed by the
  semantic scene and CPU oracle
- equivalent keyboard, IME, focus, clipboard, and accessibility outcome
- no omitted diagnostics, search results, language actions, or lifecycle work
- bounded queues and caches, terminal frame outcomes, and post-close drain

An implementation that does less work fails admission. Its timing is retained
only as rejected evidence.

## Sampling and statistics

1. Calibrate A/A variance before activating a threshold.
2. Separate cold and warm experiments.
3. Use at least four independent hardware windows and twenty paired runs before
   activating a claim, increasing repetitions when calibration requires it.
4. Randomize paired AB and BA order within each window.
5. Preserve raw samples and rejected-run records.
6. Report p50, p95, p99, effect size, and confidence interval for each metric.
7. Dimension repetition where observed uncertainty occurs rather than choosing
   one arbitrary sample count.

This follows Kalibera and Jones' guidance to account for multiple sources of
non-determinism and report effect-size confidence intervals
([Kalibera and Jones, 2013](https://kar.kent.ac.uk/33611/)).

## Memory protocol

Record Alpine-owned allocated and retained bytes exactly. Record process
physical footprint, private dirty memory, allocator counts, mapped files, GPU
resource bytes, each cache's current and peak bytes, peak footprint,
steady-state slope, and post-close delta. Sample at defined semantic points,
not arbitrary wall-clock offsets.

A bounded cache still fails if total physical footprint grows without a stable
plateau. A stable process footprint does not excuse incorrect Alpine-owned
accounting. Both evidence classes are required.

## Invalid runs

Reject and retain the reason for any run with workload mismatch, semantic or
accessibility failure, omitted behavior, stale background result, dropped or
unpresented frame, thermal or display drift, unexpected process, missing raw
sample, hash mismatch, cache-budget violation, post-close leak, or statistically
inconclusive result.

## Claim grammar

Accepted claims name comparator, revision, workload, endpoint, environment,
sample class, effect, confidence interval, and non-inferiority margins. Example:

> On environment hash H, Alpine revision A had lower p99 commit-to-presented
> latency than GPUI revision Z for workload W, while all semantic and residency
> gates passed and no matched metric crossed its accepted regression margin.

Forbidden claims include "fastest UI framework", "zero latency", one-number
aggregate scores, editor-only evidence generalized to all UI, and results that
mix adaptation with renderer timing.
