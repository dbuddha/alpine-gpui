# Alpine Studio release profiling

This protocol captures diagnostic evidence for Defect #304. It does not activate
a latency threshold and does not qualify a comparison claim.

The build-only compiler adapter, exact transitive closure, rejected alternatives,
and static footprint evidence are retained in [Decision #311](https://github.com/dbuddha/alpine-gpui/issues/311).

## Instrumentation boundary

The release application emits `com.dbuddha.alpine-studio` signposts in Apple's
`DynamicTracing` category. The category is disabled unless a performance tool is
recording. Alpine initializes the process-lifetime log before entering AppKit,
checks whether dynamic tracing is enabled once, emits only copied integers with
static names, and retains no samples. Instruments owns all raw trace storage.

Every point carries the native event timestamp, scene revision, runtime document
revision, buffer revision, and three stage-specific values:

| Point | `a` | `b` | `c` |
| --- | --- | --- | --- |
| Event Dispatch Begin | event kind | selection revision | reserved |
| State Mutation Complete | visual change | document change | selection revision |
| Frame Build Begin | viewport width bits | viewport height bits | reserved |
| Visible Layout Complete | active rendered lines | reserved | reserved |
| Text Summary | shaping calls | confirmed-miss rasterizations | cache-shaped lines |
| Layout Cache Summary | hits | misses | retained bytes |
| Glyph Atlas Summary | hits | misses | CPU atlas bytes |
| Atlas Publication Begin | pending glyphs | reserved | reserved |
| Atlas Publication Complete | 0 none, 1 full, 2 rows | payload bytes | payload groups |
| Frame Build Complete | paint operations | glyph instances | 1 for fallback |
| Native Event Handler Latency | elapsed nanoseconds | reserved | reserved |
| Native Frame Queue Latency | elapsed nanoseconds | reserved | reserved |
| Native Submission Latency | elapsed nanoseconds | reserved | reserved |
| Native GPU Terminal Observed Latency | event-to-observer nanoseconds | reserved | reserved |
| Native Presented Handler Latency | event-to-callback nanoseconds | reserved | reserved |
| Native Terminal Record Latency | event-to-record nanoseconds | reserved | reserved |
| Native Display Link Target Latency | event-to-display-link target nanoseconds | reserved | reserved |
| Native Target Presentation Latency | event-to-target-presentation nanoseconds | reserved | reserved |
| Native Actual Presentation Latency | event-to-drawable-presentation nanoseconds | reserved | reserved |
| Native Presentation Callback Lag | actual-presentation-to-callback nanoseconds | reserved | reserved |

`Atlas Publication Failed` and `Frame Build Failed` are terminal omission points.
Terminal native latency points are emitted together when the corresponding frame reaches
terminal evidence. Presentation points are emitted directly from the one-shot
drawable callback.
Their `a` payload contains the process-monotonic duration;
their trace event timestamp is not substituted for the measured endpoint. Every
native point uses the producing event identity and zero scene, document, and
buffer revisions, so it joins to Studio scene points by event identity without
inventing unavailable revision ownership. Optional native stages are omitted
when unavailable. `Native GPU Terminal Observed Latency` remains an upper bound
from main-thread observation, not exact GPU execution time. The native frame
terminal snapshot remains authoritative for request, commit, actual presentation
timestamp, outcome, retained bytes, and recovery classification.

Profile vocabulary v2 anchors event receipt with CACurrentMediaTime, the same
monotonic host clock used by CAMetalDisplayLinkUpdate.targetTimestamp,
targetPresentationTimestamp, and MTLDrawable.presentedTime. It reports the
display-link target, target presentation, actual presentation, callback lag,
and callback arrival separately. Zero, non-finite, backward, overflowing, or
closing-generation clocks are omitted rather than coerced. The v1 analyzer and
retained v1 packages remain byte-stable and are never reinterpreted as v2.

## Opt-in persisted fallback

When full Xcode is unavailable, the exact process-start opt-in
`ALPINE_STUDIO_PERSISTED_PROFILE=1` mirrors the same points into macOS unified
logging under subsystem `com.dbuddha.alpine-studio` and category
`PersistedProfile`. The dynamic Instruments route remains unchanged and can run
at the same time. Alpine samples the opt-in once, creates the persisted log
handle lazily on first emission, uses static format strings and copied integers,
and retains no profile history or file handle. Absent, empty, non-Unicode, and
non-`1` values leave the fallback disabled.

Use the canonical release bundle and record the exact start and end wall-clock
times before interacting with Studio:

```sh
scripts/build-alpine-studio-app.sh
APP="$PWD/target/release/Alpine Studio.app/Contents/MacOS/alpine-studio"
START_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)
START_LOCAL=$(date '+%Y-%m-%d %H:%M:%S')
ALPINE_STUDIO_PERSISTED_PROFILE=1 "$APP" WORKLOAD_PATH
# Perform exactly one accepted workload, close Studio, then record both end times.
END_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)
END_LOCAL=$(date '+%Y-%m-%d %H:%M:%S')
log show \
  --start "$START_LOCAL" \
  --end "$END_LOCAL" \
  --style json \
  --predicate 'subsystem == "com.dbuddha.alpine-studio" && category == "PersistedProfile"' \
  > persisted-profile.json
```

Retain the unmodified JSON output with the capture manifest. A valid record has
the static stage name plus correlation, event, scene, document, buffer, and
three numeric values. Missing stages remain omissions and are never filled with
zero. The persisted fallback does not provide Time Profiler stacks, System
Trace scheduling, Metal System Trace, exact GPU execution duration, or optical
latency, so those requirements remain open.

Validate and summarize each raw capture before interpreting it. Record the
capture host's exact `mach_timebase_info` numerator and denominator; they are
required to convert unified-log `machTimestamp` differences into nanoseconds.

```sh
scripts/analyze-studio-profile-v2.sh \
  persisted-profile.json \
  target/qualification/studio-profile-analysis \
  MACH_TIMEBASE_NUMER \
  MACH_TIMEBASE_DENOM
```

The v2 analyzer rejects mixed process identity, unknown or duplicate stages,
malformed static messages, correlation drift, decreasing timestamps, and
invalid revision ownership. It emits byte-preserved record identity,
`samples.tsv`, `summary.tsv`, `counters.tsv`, and `omissions.tsv`. Percentiles
use deterministic nearest-rank selection. The report always records
`observer_cost_calibrated=false`, `causal_attribution_allowed=false`, and
`threshold_activation_allowed=false`; a separate accepted calibration package
must change those claim boundaries rather than editing analyzer output.

## Retained diagnostic checkpoint

Task [#371](https://github.com/dbuddha/alpine-gpui/issues/371) retains the
process-filtered physical capture that informed the dropped-presentation
correction in [PR #367](https://github.com/dbuddha/alpine-gpui/pull/367).
The canonical package is
[`assurance/studio-profile/v1/c93950e-physical-diagnostic`](../../assurance/studio-profile/v1/c93950e-physical-diagnostic/README.md).
It binds the original raw hash while committing only a privacy-normalized,
analyzer-minimal record stream. Repository validation replays the version 1
analyzer and requires byte-identical summaries, samples, counters, omissions,
and claim boundaries.

The checkpoint has zero presented-handler samples. It supports the regression
that optional presentation telemetry cannot own frame-slot release or editor
progress, but it does not attribute the remaining queue or GPU-observer tails.
Observer A/A calibration, Instruments traces, display refresh, power, thermal,
and optical evidence are absent. Defect #304 and Experiment #331 therefore
remain open.

### Command-buffer presentation negative result

[Experiment #553](https://github.com/dbuddha/alpine-gpui/issues/553) tested a
command-buffer-owned drawable presentation against the accepted direct-present
path on the same Apple M4 host. The canonical paired package is
[`assurance/studio-profile/v2/553-command-buffer-presentation`](../../assurance/studio-profile/v2/553-command-buffer-presentation/README.md).
Both isolated runs preserved the exact `33,333,250 ns` interval from the
display-link target to target presentation. The candidate's `0.810 ms` lower
actual-presentation p50 was already present at the display-link target and is
not attributable to command-buffer ownership under the uncalibrated one-pair
protocol. The candidate was rejected, the direct-present path remains current,
and the fixed presentation offset remains open under #304 and #331.

Unified logging has observer cost. Run matched capture-off and capture-on A/A
windows before using persisted distributions to attribute a stall. Reject the
fallback for causal analysis when its confidence interval shows material
latency, queue, frame-deadline, CPU, memory, or energy distortion. Even after a
clean A/A result, persisted samples remain diagnostic until the physical
release workload, environment identity, raw log, executable hash, revision,
and invalidation checks are retained. The fallback alone does not activate a
threshold or close Defect #304.

## Capture preflight

Use a clean revision and the canonical release bundle. Full Xcode must be the
active developer directory because Command Line Tools alone do not contain
`xctrace`.

```sh
xcrun --find xctrace
xcodebuild -version
scripts/build-alpine-studio-app.sh
test -x 'target/release/Alpine Studio.app/Contents/MacOS/alpine-studio'
```

Record each tool separately so its observer cost is not mixed with another
instrument. Replace `WORKLOAD_PATH`, `TRACE_ROOT`, and the time limit with the
accepted workload values.

```sh
WORKLOAD_PATH="$PWD"
TRACE_ROOT="$PWD/target/qualification/studio-$(git rev-parse HEAD)"
APP="$PWD/target/release/Alpine Studio.app/Contents/MacOS/alpine-studio"
mkdir -p "$TRACE_ROOT"

xcrun xctrace record \
  --template 'Time Profiler' \
  --output "$TRACE_ROOT/time-profiler.trace" \
  --time-limit 60s \
  --launch -- "$APP" "$WORKLOAD_PATH"

xcrun xctrace record \
  --template 'System Trace' \
  --output "$TRACE_ROOT/system-trace.trace" \
  --time-limit 60s \
  --launch -- "$APP" "$WORKLOAD_PATH"

xcrun xctrace record \
  --template 'Metal System Trace' \
  --output "$TRACE_ROOT/metal-system-trace.trace" \
  --time-limit 60s \
  --launch -- "$APP" "$WORKLOAD_PATH"
```

Template names must be confirmed with `xcrun xctrace list templates` on the
capture machine. A missing or renamed template invalidates that capture; it is
not silently replaced by a different instrument.

## Required manifest and workloads

Retain the Alpine revision and executable SHA-256, dirty state, workload hash,
macOS build, Xcode and `xctrace` versions, machine and display identities, refresh
mode, font and settings identity, power source, thermal state, warmup, start and
end times, sample count, and every raw `.trace` artifact. Record normal code
typing, burst typing, Unicode, IME composition, caret movement, selection,
completion visibility, and typing while scrolling as separate workloads.

Derive p50, p95, and p99 offline. Keep stage omissions explicit. Reject a run for
data loss, semantic mismatch, stale identity, missing raw evidence, dropped
behavior, display or power drift, thermal drift, or an unrecorded environment
change. Hosted timing remains diagnostic only. Fixed-hardware thresholds require
A/A calibration and a separate accepted activation record.
