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

`Atlas Publication Failed` and `Frame Build Failed` are terminal omission points.
Native latency points are emitted together when the corresponding frame reaches
terminal evidence. Their `a` payload contains the process-monotonic duration;
their trace event timestamp is not substituted for the measured endpoint. Every
native point uses the producing event identity and zero scene, document, and
buffer revisions, so it joins to Studio scene points by event identity without
inventing unavailable revision ownership. Optional native stages are omitted
when unavailable. `Native GPU Terminal Observed Latency` remains an upper bound
from main-thread observation, not exact GPU execution time. The native frame
terminal snapshot remains authoritative for request, commit, actual presentation
timestamp, outcome, retained bytes, and recovery classification.

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
