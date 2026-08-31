# Adversarial direction review

## Executive verdict

The architectural direction is strong. The execution system is at risk of
optimizing evidence machinery and narrow invariants faster than it closes the
user-visible daily-driver gate. The correct next move is not more framework
breadth. It is to eliminate typing lag, finish physical M4 correctness, complete
the remaining Rust/configuration slice, dogfood, and produce realistic E4
comparisons.

## Findings ordered by severity

### P0: visible typing latency invalidates the primary product promise

[Defect #304](https://github.com/dbuddha/alpine-gpui/issues/304) reports visible
event-to-present lag in the release application. This blocks daily-driver
acceptance and every smoothness or 120 Hz claim. Signposts and stage correlation
exist, but an external physical trace and root-cause proof do not.

Required correction:

- Capture native event arrival, state mutation, scene build, display-link wake,
  encode, commit, completion, presented handler, and optical response.
- Identify whether delay is queue admission, run-loop mode, display-link wake,
  scene/text work, drawable scheduling, presentation, or measurement artifact.
- Fix the measured dominant stage only.
- Add the reproduced incident as a regression workload.

### P0: no realistic comparator supports the stated framework ambition

At the time of this review, the Zed lab had one solid-quad correctness workload.
Timing and memory were intentionally disabled, so it could not support glyph,
text viewport, clipping, scroll, resize, latency, residency, or framework
dominance statements.

Status update, 2026-08-30: PR #343 and Alpine Zed Lab PR #5 provide composed E3
correctness for an eight-fixture ladder covering the immutable control,
clipping, glyph atlas sampling, a realistic code viewport, scroll, and resize.
Alpine PR #469 and Alpine Zed Lab PR #6 additionally retain the six-step atlas
lifecycle and recovery protocol as composed E3 evidence. Timing, adaptation,
process and GPU residency, independent hardware windows, and dominance remain
open under Requirement #53 through #470, #471, and #472.

Required correction:

- Retain the accepted clipped grids, glyph grids, realistic code viewport,
  selection/caret, scroll, resize, and lifecycle traces as semantic admission.
- Establish semantic equivalence before timing.
- Measure adaptation separately from framework scene build and renderer stages.
- Tasks #61, #334, and #353 are complete E3 admission. Satisfy Requirement #53
  through #470, #471, and #472 before making renderer dominance claims.

### P1: M4 physical correctness is not closed

AppKit accessibility implementation is substantial, and the production process
journey #272 is complete. Physical VoiceOver, AXObserver, and lifecycle
acceptance remain open under #223, #253, and #273. Snapshot tests cannot prove
VoiceOver behavior, external AX tree visibility, announcements, focus transfer,
or lifecycle recovery.

Required correction: run the real bundled process through AXObserver and
VoiceOver journeys on an unlocked Apple Silicon session, retaining revision and
OS identity.

### P1: project truth is inconsistent

PR #306 and #313 implement bundle and launch behavior, and #303 is complete.
Several parent requirements and old foundational tasks remain open after their
core implementation landed. Milestone issue counts therefore understate
progress and obscure true blockers.

Required correction:

- Preserve #303's accepted Finder-launch evidence and reconcile later status
  changes through #225.
- Reclassify implementation-complete parent tasks as qualification parents when
  only hardware evidence remains.
- Keep one Prototype Readiness view based on blocking leaf issues.

### P1: existing research summaries can become stale silently

The earlier Zed GPUI study described a blocking presentation callback after
Alpine had already replaced it with async slots. Narrative documents without
an implementation revision and review trigger can misdirect future agents.

Required correction: this package pins Alpine state, separates comparator and
current upstream, and requires a ledger/history update in architecture or
performance PRs. Case studies should link here rather than restating mutable
Alpine status.

### P1: 120 Hz is being discussed imprecisely

An editor should render zero frames when idle. "120 FPS" is not the universal
target. During active typing and scrolling on a 120 Hz display, p99 main-thread
work should fit inside 8.33 ms and event-to-present should meet a calibrated
deadline without systematic misses. Actual presentation and optical latency
must be separated.

Required correction: report deadline adherence, missed frames, event-to-submit,
submit-to-present, and event-to-photon distributions for named active workloads.

### P1: exact Alpine-owned accounting is necessary but insufficient

Hard cache and queue caps are excellent correctness properties. They do not
capture allocator fragmentation, AppKit/CoreText caches, Metal driver residency,
mapped buffers, process private dirty memory, or post-close drain.

Required correction: combine exact counters with physical footprint, allocator
samples, VM regions, GPU bytes, cache churn, slope, pressure recovery, and
post-close baseline.

### P2: Studio risks becoming a monolith before a justified element layer exists

The application crate has a large source and test surface with substantial
manual routing and painting. Extracting a broad GPUI clone now would be worse,
but leaving every repeated overlay, focus, geometry, and virtual-list contract
inside Studio will slow delivery and invite inconsistencies.

Required correction: profile and dogfood first. Then extract only repeated
layout/prepaint/paint, focus routing, overlay, and virtual-list contracts with
behavior-preserving ports and allocation gates.

### P2: assurance breadth can become test theater and delivery drag

TLA+, Kani, Miri, mutation, property tests, native E2E, and hardware lanes are
valuable when tied to a risk. They are harmful when synthetic bypasses replace
production behavior, mutants test trivia, or every change pays broad unrelated
cost.

Required correction:

- Keep risk classification and production-path controls.
- Track escaped defects per assurance lane.
- Delete or narrow checks whose mutation sensitivity does not protect a user or
  ownership invariant.
- Run long mutation/fuzz/soak jobs nightly while keeping PR gates narrow.

### P2: upstream drift is separated but not yet operationally closed

Zed stable moved from v1.15.0 to v1.17.2 and WGPU to v30.0.1. Keeping the
comparator immutable is correct. The prior bounded source-delta reviews #95,
#96, #100, and #302 are complete. The active review cycle is #445 through #449;
its findings must become bounded research decisions, defects, or experiments
rather than silently widening the product.

Required correction: review only changed mechanisms relevant to Alpine, record
adopt/reject/no-change decisions, and close the narrative research independently
of any resulting experiment.

### P2: no legal or technical wording should imply a Zed application fork

GPUI's framework license and Zed application's GPL boundary are different.
Alpine is an independent implementation informed by source research. "Adapting
Zed Editor" can be misread as source derivation.

Required correction: use the taxonomy in [methodology.md](methodology.md), keep
GPL source in the lab, and require code-level provenance before any copied-code
claim.

### P3: fixed bounds may encode arbitrary policy rather than user behavior

Alpine has many explicit path, query, message, result, and cache limits. This is
better than unbounded growth, but limits can cause poor large-repo behavior or
silent quality loss if they are not surfaced and calibrated.

Required correction: expose truncation and degraded behavior, exercise accepted
large-project corpora, and revise bounds from retained workload evidence.

## What to keep

- Direct Metal as the Apple v1 shipping backend.
- Immutable scenes and safe public boundaries.
- Demand-driven invalidation and strict zero-idle default.
- Three completion-owned frame slots and no normal completion wait.
- Visible-range text work and two-frame reuse.
- Lookup-before-rasterize and row-delta atlas transport.
- Local-only buffer, bounded workers, revision-tagged results.
- Exact ownership accounting paired with structured errors.
- Correctness admission before performance measurement.
- GPUI/WGPU/Zed isolation from shipping dependencies.
- AI, collaboration, cloud, telemetry, plugins, remote, debugger, terminal, and
  Git exclusions from M5.

## What to change now

1. Use the event-to-present reporting completed in #314 to resolve visible
   typing-latency defect #304 through physical stage-distribution experiment
   #331.
2. Finish #223, #253, and #273 on physical hardware; #272 already proves the
   production process journey.
3. Keep #303 and other implementation-complete issues reconciled with their
   retained evidence through #225.
4. Preserve completed behavior #218 through #222 through dogfood regressions
   instead of reporting those slices as pending.
5. Preserve completed capture task #238, then execute #239 through #242 as the
   remaining daily-driver acceptance chain.
6. Use #61/#334/#353 retained E3 evidence as admission, then satisfy #470,
   #471, and #472 under #53 before timing or memory claims.
7. Run WGPU experiments only after those trace semantics stabilize.
8. Update this ledger in every material architecture/performance PR.

The assurance repair cycle represented by #401, #406, and #443 is complete.
Live Studio dogfood diagnostics and bounded capture are implemented through
#461 and #462. Exact-head and exact-main aggregate CI remain the authoritative
admission evidence for future changes.

## Wrong-direction triggers

Stop and require a new accepted requirement if work introduces:

- A GPUI compatibility API or retained entity graph before dogfood evidence.
- WGPU, Naga, WGSL, Tokio, or a game-engine runtime in shipping crates.
- General CSS, DOM, render graph, ECS, animation, or asset-pipeline machinery.
- Terminal, Git, debugger, plugin, AI, collaboration, cloud, or remote scope into
  M5.
- A performance threshold before A/A calibration and correctness admission.
- A universal fastest-framework or 120-FPS claim from editor-only evidence.

## Final adversarial assessment

Alpine has not gone off track in architecture. It has gone too long without
closing the hardest user-visible proof: typing must feel immediate in the real
release application. The strongest path is now ruthless prioritization, not
more research breadth. Research should directly unblock traces, profiling,
residency, or a product decision and then close.
