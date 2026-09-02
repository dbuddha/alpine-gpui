# Command-buffer presentation experiment

This package retains the physical matched-pair evidence for
[Experiment #553](https://github.com/dbuddha/alpine-gpui/issues/553), under
[Defect #304](https://github.com/dbuddha/alpine-gpui/issues/304),
[Experiment #331](https://github.com/dbuddha/alpine-gpui/issues/331), and
[Task #522](https://github.com/dbuddha/alpine-gpui/issues/522).

The experiment tested whether replacing Alpine's independent
`MTLDrawable::present` call with `MTLCommandBuffer::presentDrawable` reduced the
measured presentation tail. It did not provide evidence to adopt the change.
The candidate commit remains rejected and is not part of this package's source
branch.

## Matched protocol

Both runs used the same Apple M4 host, built-in composited display, AC power,
55-byte ASCII workload, isolated `HOME`, empty starting file, Computer Use input,
persisted profile-v2 route, semantic file verification, visual inspection,
process-filtered unified log, and production close. Each executable is bound by
source revision and SHA-256. Candidate and baseline text rendered upright and
byte-identically. Both processes exited cleanly.

The original unified logs are identified but not committed because every record
contains private absolute paths. `records.json.gz` retains only analyzer-required
fields and replaces the boot identity and executable path with disclosed stable
placeholders.

## Result

| Metric | Baseline p50 | Candidate p50 | Candidate minus baseline |
| --- | ---: | ---: | ---: |
| Display-link target | 15.161 ms | 14.366 ms | -0.795 ms |
| Target presentation | 48.494 ms | 47.699 ms | -0.795 ms |
| Actual presentation | 48.508 ms | 47.699 ms | -0.810 ms |
| Presentation callback lag | 0.193 ms | 0.179 ms | -0.014 ms |
| Native submission | 0.095 ms | 0.079 ms | -0.016 ms |
| Frame build | 0.054 ms | 0.052 ms | -0.002 ms |

The display-link-target to target-presentation interval is exactly
`33,333,250 ns` in both runs. The candidate's apparent actual-presentation shift
is already present at the display-link target, leaving only `14,250 ns` of
residual change between target and actual presentation. This one paired E3
window has uncalibrated observer cost and cannot attribute the sub-millisecond
shift to command-buffer ownership.

The candidate also did not provide favorable memory evidence. Its RSS was
higher before and after the workload, but one short uncalibrated pair is not a
residency claim in either direction.

## Decision

Reject the candidate and retain Alpine's current direct-present path. The
remaining fixed two-refresh offset is upstream of this Metal submission choice.
Follow-up experiments must evaluate display-link scheduling and a bounded
high-rate input presentation tail independently, without weakening zero-idle,
three-slot ownership, or lifecycle correctness.

This package supports only a negative E3 implementation decision. It does not
activate a latency threshold or support an Alpine, Zed, GPUI, WGPU, Sublime,
120 Hz, memory, causal, or optical-latency claim.

## Reproduce

Run:

```sh
scripts/check-studio-profile-v2-evidence.sh
```
