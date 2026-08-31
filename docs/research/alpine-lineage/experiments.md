# Missing experiments and qualification path

## Priority order

| Priority | Experiment | Blocks | Current level | Exit evidence |
| --- | --- | --- | --- | --- |
| P0 | Physical typing latency root cause | #304, private dogfood, 120 Hz claim | Diagnostic-only | Reproduced stage distribution, causal fix, regression, external trace |
| P0 | Realistic code-viewport trace semantics | #53 and every renderer claim | E3 composed static and lifecycle correctness | Calibrated timing, residency, and E4 GPUI comparison through #470-#472 |
| P1 | Physical VoiceOver and AX process journey | M4 | E3 modeled | External AX/VoiceOver evidence on production bundle |
| P1 | Warm text CPU/GPU profile | Atlas performance claim | E3 deterministic | Zero raster/upload plus measured scene, shaping, allocation, upload, GPU stages |
| P1 | Sustained Alpine repository dogfood | M5 | Implementation evidence | Revision-pinned sessions, incidents, no data loss, accepted defect window |
| P1 | Long-session residency and drain | Memory-efficiency claim | Exact counters only | Physical footprint slope, pressure recovery, close baseline |
| P2 | Paired GPUI renderer qualification | Framework dominance | No timing | E4 AB/BA distributions after semantic admission |
| P2 | Paired Zed/Sublime local-editor journeys | Product claims | Protocol only | E4 normalized and stock-product reports |
| P3 | WGPU differential backend | Contract robustness | E2 design | E3 semantic/lifecycle reproduction; timing only after equivalence |

## EXP-LAT-001: release typing event-to-photon

Workload:

- Warm release `.app`, one Rust file, fixed font and viewport.
- Single Unicode keystrokes, 20-character bursts, key repeat, IME composition,
  selection replacement, and large paste reported separately.
- 60 Hz and 120 Hz composited modes.

Stages:

- AppKit event timestamp.
- Main-thread dispatch and state revision.
- Scene build start/end.
- frame request and display-link callback.
- upload, encode, commit, GPU completion.
- drawable presented handler.
- optical photon response where available.

Controls:

- A/A variance before thresholds.
- Warm and cold runs separated.
- No background indexing or LSP in the base typing workload; add each as a
  controlled treatment.
- Invalidate locked, thermally drifting, display-changing, or identity-missing
  runs.

Acceptance:

- No systematic extra run-loop or display interval.
- Main-thread p99 under the calibrated 8.33 ms active frame budget at 120 Hz.
- No semantic loss, dropped input, stale presentation, or idle submissions.
- Root cause and regression linked to #304.

## EXP-REN-001: realistic renderer trace ladder

Add traces in this order:

1. Solid quad, retained as the protocol smoke control.
2. Clipped quad grid.
3. Monochrome glyph grid with fixed atlas.
4. Realistic Rust viewport with gutters, selections, caret, and clipping.
5. One-line scroll delta with mostly reused text.
6. Resize and scale transition.
7. Atlas miss, growth, eviction, and recovery.

As of 2026-08-30, the immutable control, clipped grid, glyph grid, realistic
code viewport, scroll pair, resize pair, and six-step atlas lifecycle pass
composed E3 CPU, Alpine Direct Metal, and pinned GPUI Metal admission. The
lifecycle covers admission, compatible reuse, content replacement, capacity
change, teardown, and clean reconstruction. Timing and adaptation calibration
#470, renderer residency #471, and independent-window E4 qualification #472
remain Requirement #53 work.

For each trace retain semantic input hash, decoded normalized scene hash, output
oracle, tolerance, adapter allocation bytes, adapter time, scene-build time,
renderer time, and omissions. Do not enable timing for a trace until both paths
produce accepted equivalent output.

## EXP-GPU-001: warm text transport

Treatments:

- Unchanged warm viewport.
- One new glyph.
- One row-spanning glyph.
- Atlas growth.
- Eviction and reuse.
- Device/buffer resynchronization.

Measure:

- Layout materialization, shaping calls, raster calls, CPU allocations.
- CPU atlas bytes changed and copied.
- staging bytes and GPU atlas bytes uploaded.
- encode/commit and GPU duration.
- physical footprint and retained capacity.

The deterministic expected values are zero warm rasterizations, publications,
and uploads; one confirmed miss rasterization; bounded affected rows; one full
replacement on growth/recovery. A performance improvement requires paired
physical measurements, not these counts alone.

## EXP-MEM-001: editor residency

Windows:

- Cold launch.
- Warm single file.
- Alpine repository after indexing.
- Repeated edit/undo.
- Scrolling and file switching.
- Search result churn.
- LSP diagnostic/completion storms.
- Hide/show and sleep/wake.
- Close and post-close drain.

Metrics:

- Physical footprint and private dirty.
- Allocator live, allocated, and peak bytes.
- Alpine scene, frame-slot, upload, atlas, layout, font, search, file-tree, LSP,
  settings, recovery, and queue bytes.
- Metal resource allocation and retained capacity.
- Steady-state slope and post-close delta.

Acceptance requires no unbounded slope, all explicit budgets observed, visible
degradation for admission limits, and accepted post-close baseline.

## EXP-FWK-001: Alpine versus pinned GPUI

Protocol:

- Comparator remains Zed `v1.15.0` until requalified.
- Run identical normalized traces after adapter completion.
- Report adapter, scene build, upload, encode/commit, GPU completion, and
  presentation separately.
- Randomize paired AB/BA order across at least four independent hardware
  windows and the calibrated sample count.
- Report p50, p95, p99, effect size, confidence intervals, missed deadlines,
  physical memory, allocator activity, and GPU bytes.

Admission:

- Semantic output, clipping, omissions, lifecycle, and accessibility-relevant
  behavior green.
- No hidden work removed from one side.
- No timing from runtime shader compilation.
- No stock-product process weight mixed into renderer-only results.

## EXP-PROD-001: Alpine Studio versus Zed and Sublime

Journeys:

- Empty window and direct file open.
- Restored Alpine repository and first accepted keystroke.
- Warm typing and Unicode editing.
- Quick open and file switching.
- Find and project search.
- Rust completion and navigation.
- Long scroll and steady idle.

Report normalized local-only and stock-product lanes separately. Sublime is
measured externally; no private internal mechanism is inferred. Product claims
name the exact journey, metric, hardware, revisions, exclusions, and confidence
interval.

## EXP-WGPU-001: differential oracle

Begin only after EXP-REN-001 trace semantics are stable. Implement traces in an
isolated lab with no WGPU shipping dependency. Compare output, clipping, glyph
sampling, resize, device loss, and resource lifetime. Use contradictions to
improve Alpine contracts. Timing and memory are optional and may start only
after semantic equivalence.

## Evidence storage

- Protocols and accepted conclusions live in Git.
- Raw samples and captures live under immutable assurance identities or release assets.
- Every report includes `workload_identity_hash`, `environment_hash`,
  `exclusion_manifest_hash`, Alpine revision, comparator revision, adapter
  revision, and invalid-run log.
- Expiring CI artifacts supplement but never replace canonical evidence.
