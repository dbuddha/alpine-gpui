# One-frame CAMetalDisplayLink latency experiment

This package retains the negative physical result for
[Experiment #544](https://github.com/dbuddha/alpine-gpui/issues/544), under
typing Defect [#304](https://github.com/dbuddha/alpine-gpui/issues/304) and
physical Experiment [#331](https://github.com/dbuddha/alpine-gpui/issues/331).
It is diagnostic evidence, not a threshold, causal profile, optical
measurement, or comparator claim.

## Question

Would changing CAMetalDisplayLink.preferredFrameLatency from 2.0 to 1.0
reduce Alpine Studio's scheduled presentation lead in a normal composited
120 Hz macOS window?

Apple documents 1.0 and 2.0 as the only accepted values and warns that the
final latency may be larger in windowed macOS. Alpine continued to use the
display-link callback drawable, direct present(), three bounded frame slots,
asynchronous completion, demand-driven invalidation, and zero clean-idle work.

## Matched physical method

Both release executables came from the recorded source revisions and ran on the
same Apple M4 host, macOS build, built-in 120 Hz display, power state, and
process-start persisted-profile route. LaunchServices received a capture-local
HOME, and each run opened only one identical file.

The Computer Use workload performed normal typing at a 35 ms cadence, burst
typing, caret movement, selection replacement, an exact Unicode clipboard
paste, atomic save, and production close. Both runs produced the same final
document SHA-256 and exited cleanly.

## Result

| Policy | Complete target and actual samples | Target presentation minus display-link target |
| --- | ---: | ---: |
| preferredFrameLatency = 2.0 | 42 | exactly 33.333250 ms for every sample |
| preferredFrameLatency = 1.0 | 44 | exactly 33.333250 ms for every sample |

The candidate produced no scheduling reduction and is rejected. Production
remains at preferredFrameLatency = 2.0. Small differences in event-to-target,
event-to-actual, or callback tails are not causal evidence because persisted-log
observer cost has not been calibrated and this package is one matched window,
not a qualified statistical comparison.

This result narrows Defect #304: ordinary state mutation, frame construction,
submission, and callback dispatch are not the fixed 33.333250 ms component.
The component is the display-link target-to-target-presentation interval
observed in this windowed mode. A future correction must use a separately
accepted and bounded pacing experiment. It must not add a continuous game loop,
unbounded frame queue, idle rendering, or unsupported presentation method.

## Identity and privacy

manifest.toml binds both source revisions, signed executables, original
route-filtered raw hashes, normalized record hashes, workload, analyzer,
capture windows, host, display, power, thermal state, and every derived file.
The original unified-log files are identified but not committed because each
record repeats a local absolute executable path.

Each compressed record stream retains only the version 2 analyzer fields.
Boot identity and executable path use disclosed stable placeholders. User ID,
wall-clock timestamp, sender path, backtrace, thread identity, trace identity,
and unrelated process logs are removed. Static event messages contain numeric
identities and counters, not document text, paths, or keystrokes.

## Reproduce

Run:

~~~sh
scripts/check-studio-profile-v2-evidence.sh
~~~

The gate verifies exact files, privacy, hashes, claim ceilings, source identity,
and matched workload identity. It decompresses both record streams, reruns the
version 2 analyzer, byte-compares every canonical output, derives presentation
pairs again, and requires the recorded no-change result. Its negative controls
reject artifact drift, private paths, derived-table drift, relaxed claim
boundaries, and a fabricated scheduling improvement.

The package does not close #304 or #331. Observer A/A calibration, IME,
completion visibility, typing while scrolling, Instruments traces, residency,
optical latency, and comparator qualification remain open.
