# Physical dropped-presentation diagnostic

This Task [#371](https://github.com/dbuddha/alpine-gpui/issues/371)
package retains the physical Alpine Studio capture that informed
[PR #367](https://github.com/dbuddha/alpine-gpui/pull/367) and
[Defect #304](https://github.com/dbuddha/alpine-gpui/issues/304).
It is diagnostic evidence, not a threshold, causal profile, optical latency
measurement, or comparator claim.

## Identity and privacy

`manifest.toml` binds the correction source, executable, workload, analyzer,
capture window, physical host, original raw hash, normalized record hash, and
all derived artifacts. The original macOS unified-log JSON is identified but is
not committed because it repeats the local username and absolute executable
path in every record.

`records.json.gz` retains only fields required by the version 1 analyzer. The
boot identity and executable path use disclosed stable placeholders. User ID,
wall-clock timestamp, source path, sender path, backtrace, thread, trace, and
other OS metadata are removed. Event messages use Alpine's static numeric
grammar and contain no document text, path, keystroke, or source content.

## Observed diagnostic distribution

| Metric | Count | p50 | p95 | p99 |
| --- | ---: | ---: | ---: | ---: |
| State mutation | 904 | 1.916 us | 13.166 us | 20.708 us |
| Frame build | 78 | 84.750 us | 224.916 us | 949.625 us |
| Native event handler | 23 | 65.375 us | 208.000 us | 238.583 us |
| Native frame queue | 23 | 155.000 us | 23.073 ms | 59.775 ms |
| Native submission | 23 | 98.667 us | 193.709 us | 644.042 us |
| GPU terminal observation | 23 | 8.636 ms | 34.092 ms | 68.273 ms |

There are zero presented-handler samples and 347 explicit stage omissions.
The missing endpoint cannot be filled from a display-link target timestamp.
The production correction terminates a dropped presentation, releases its frame
slot, and does not replay the same immutable event identity.

The queue and GPU-observer tails remain unexplained. Persisted logging observer
cost is not calibrated. The 332 ms Computer Use call for one 72-character burst
is transport elapsed time, not input-to-present or optical latency. Full Xcode
Instruments, A/A windows, separate 60 and 120 Hz journeys, and causal profiling
remain open under #304 and #331.

## Reproduce the retained analysis

Run:

```sh
scripts/check-studio-profile-evidence.sh
```

The gate verifies privacy and artifact hashes, decompresses the retained record
stream, invokes `scripts/analyze-studio-profile.sh`, and byte-compares the
report, summary, samples, counters, and omissions. Its negative controls prove
that hash drift, a private path, and a self-consistent but incorrect derived
table all fail.
