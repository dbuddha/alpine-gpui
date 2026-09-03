# Renderer stage attribution

## Purpose

Task [#521](https://github.com/dbuddha/alpine-gpui/issues/521) tracks a
realistic code viewport where the pinned GPUI renderer completed synchronous
offscreen submit-readback faster than Alpine. Aggregate elapsed time could not
identify whether Alpine was spending that time in scene admission, native
resource preparation, command encoding, host synchronization, GPU execution,
or CPU readback.

Alpine now exposes an explicit `render_offscreen_profiled` path and a
`profile-scene-native` assurance command. The ordinary `render_offscreen` path
returns no timing evidence and contains no active timing probes. This separation
keeps the existing GPUI comparator path probe-free.

The stage profile records:

- Scene admission and lowering.
- Native resource and upload preparation.
- Command-buffer acquisition.
- Atlas upload, render-pass, and readback encoding.
- Command commit.
- Host completion waiting.
- Metal `GPUStartTime` to `GPUEndTime` when available.
- CPU readback compaction.
- Submission accounting and total elapsed time.

Metal GPU execution overlaps host completion waiting. Those values must not be
added together.

## First physical diagnostic

The first retained package is
`assurance/qualification/v2/raw/issue-521-388e4b8/`. Its manifest binds the
source revision, release binary, realistic viewport trace, raw CSV files,
environment, commands, and hashes. It is E3 diagnostic evidence from one
physical window on an Apple M4 Mac16,1 with 24 GB of memory and macOS 26.6.2.
Every admitted, warmup, and measured frame retained exact BGRA8 output.

The capture used 100 warmups and 100 measured frames:

| Stage | p50 ns | p95 ns | p99 ns |
| --- | ---: | ---: | ---: |
| Admission | 167 | 209 | 500 |
| Resource preparation | 10,333 | 14,000 | 23,000 |
| Command-buffer acquisition | 375 | 459 | 625 |
| Atlas upload encoding | 0 | 0 | 0 |
| Render encoding | 5,500 | 5,875 | 6,750 |
| Readback encoding | 4,959 | 5,291 | 5,625 |
| Commit | 2,042 | 3,583 | 5,667 |
| Host completion wait | 336,000 | 376,167 | 560,167 |
| Metal GPU execution | 34,667 | 35,458 | 37,208 |
| Readback compaction | 458 | 542 | 1,000 |
| Profiled total | 362,375 | 408,375 | 591,625 |
| Ordinary probe-free total | 315,125 | 421,625 | 499,375 |

## Interpretation boundary

This package supports three scoped findings:

1. Shader and GPU execution are not the first optimization target for this
   warm viewport on this revision.
2. The profiled path spends most observed time waiting for synchronous terminal
   completion, while resource preparation and command encoding are much
   smaller components.
3. Profiling perturbs the distribution, so profiled and ordinary totals cannot
   be subtracted to estimate a causal probe cost or renderer overhead.

The package does not prove why GPUI is faster, that Alpine regressed, or that
changing synchronization would preserve the synchronous readback contract. It
contains no confidence interval and authorizes no performance claim.

## Required next evidence

1. Randomize ordinary and profiled A/A order across independent physical
   windows to quantify probe perturbation.
2. Capture equivalent pinned GPUI stages or the nearest source-valid boundaries
   without placing adaptation inside renderer timing.
3. Measure reusable texture, readback, and upload ownership independently.
4. Preserve exact CPU-oracle, Alpine, and GPUI output equivalence.
5. Rerun the paired cross-renderer protocol only after a measured correction.
6. Accept a result only with renderer residency and independent-window gates
   #471 and #472.

No Studio journey, optical latency, universal framework claim, or shipping GPUI
or WGPU dependency is part of this experiment.
