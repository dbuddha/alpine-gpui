---
title: WGPU architecture and qualification case study
status: accepted-research
reviewed: 2026-08-18
historical_revision: ee5cfb074fd0c4e318b5f8608df504678e4e17ac
reviewed_revision: 8ee190c6f151c731a4f8cfd9a102d6ee5903460a
release_context: v30.0.0
research_issues: [23, 99]
implementation_task: 202
---

# WGPU architecture and qualification case study

This case study replaces the former four-bullet WGPU summary. It is the
decision-facing synthesis of the evidence retained in the
[WGPU research package](../research/wgpu/index.md). It answers one narrow
question: which WGPU engineering practices improve Alpine's correctness,
performance, memory discipline, and delivery, without importing WGPU's broader
portability problem into the Apple-first v1 renderer?

## Research identity

| Field | Value |
| --- | --- |
| Historical Alpine pin | [`ee5cfb074fd0c4e318b5f8608df504678e4e17ac`](https://github.com/gfx-rs/wgpu/tree/ee5cfb074fd0c4e318b5f8608df504678e4e17ac), reviewed 2026-08-13 |
| Current review pin | [`8ee190c6f151c731a4f8cfd9a102d6ee5903460a`](https://github.com/gfx-rs/wgpu/tree/8ee190c6f151c731a4f8cfd9a102d6ee5903460a), reviewed 2026-08-18 |
| Release context | [`v30.0.0`](https://github.com/gfx-rs/wgpu/releases/tag/v30.0.0), published 2026-07-01 |
| Upstream records | [Research #23](https://github.com/dbuddha/alpine-gpui/issues/23), [re-evaluation #99](https://github.com/dbuddha/alpine-gpui/issues/99) |
| Implementing record | [Task #202](https://github.com/dbuddha/alpine-gpui/issues/202) |
| License | Apache-2.0 or MIT by repository policy; file-level review remains mandatory before adaptation |
| Evidence grade | Source-verified architecture and contract analysis. No Alpine-to-WGPU performance experiment has yet run. |

No WGPU source or dependency is incorporated by this research.

## Executive decision

WGPU is factored into Alpine in three roles:

1. A primary-source specimen for validation, resource lifetime, submission,
   surface recovery, staging reuse, and layered GPU testing.
2. A future non-shipping differential oracle for matched offscreen scenes,
   after a separate dependency and experiment approval.
3. A portability option that may be reconsidered after the direct-Metal daily
   driver is qualified, never a constraint on the Apple Silicon fast path.

WGPU is not a v1 shipping dependency, not Alpine's scene model, not its native
surface abstraction, and not evidence that portability overhead is free. Its
architecture intentionally solves WebGPU conformance, untrusted-input
validation, multiple native backends, browsers, FFI, and shader translation.
Alpine v1 solves one controlled native editor workload on Metal.

## Findings that change Alpine work

| ID | Source-supported finding | Alpine action |
| --- | --- | --- |
| CS-WGPU-001 | WGPU separates idiomatic API, validated core, unsafe portable HAL, and native backends. The layers have distinct safety duties. | Keep Alpine's safe public contracts separate from native unsafe code, but do not reproduce WGPU's backend-neutral core. |
| CS-WGPU-002 | Submission indices and a lifetime tracker retain resources until GPU completion and order mapping callbacks before work-done callbacks. | Keep Alpine's three-slot generation and frame-token model, and add model cases for completion reorder, stale completion, close, and mapping-equivalent callback order. |
| CS-WGPU-003 | WGPU's Metal path commits without a completion wait during normal present. `waitUntilCompleted` appears on explicit idle waiting, while transaction presentation waits only until scheduled. | Preserve Alpine's non-blocking display callback. Never reintroduce GPU completion waits on the frame-admission path. |
| CS-WGPU-004 | Metal surface configuration derives maximum drawable count from frame latency, and current code skips acquisition for occluded macOS windows to avoid a documented `nextDrawable` stall. | Retain Alpine's three-drawable ceiling, visibility gate, and no-acquire behavior while hidden or occluded. Add a native regression workload for occlusion transitions. |
| CS-WGPU-005 | Surface acquisition returns structured success, suboptimal, timeout, occluded, outdated, lost, and validation outcomes. | Preserve structured Alpine presentation errors and define an explicit recovery table rather than panic or retry loops. |
| CS-WGPU-006 | `StagingBelt` amortizes many small uploads through reusable chunks, but its own contract warns that unsubmitted closed chunks can cause indefinite allocation. | Keep Alpine's bounded reusable upload slots, expose retained capacity, and test abandoned or failed submissions as memory hazards. Do not copy the general belt API. |
| CS-WGPU-007 | WGPU separates fast no-GPU validation tests, real-GPU tests, tolerant image comparison, compile tests, dependency-tree tests, shader snapshots, and WebGPU CTS. | Add the no-device validation pattern where Alpine native state can be modeled, keep real Metal E2E for platform truth, and add a shipping dependency-tree exclusion gate. |
| CS-WGPU-008 | WGPU explicitly says its older trace/player tests are difficult to author and currently broken, while its validation and GPU harnesses are preferred. | Do not equate trace replay with qualification. Keep Alpine trace identity, but require semantic oracle, native evidence, and workload controls independently. |
| CS-WGPU-009 | WGPU core performs broad validation, state tracking, zero initialization, barrier generation, cross-device checks, and resource retention because it accepts generalized and potentially untrusted WebGPU usage. | Copy the fail-closed discipline, not the generalized machinery. Alpine should encode only the states reachable through its owned scene and renderer. |
| CS-WGPU-010 | The reviewed four-day upstream delta moved IDs, registries, and `Global` into `wgpu-core-remote` and changed synchronization primitives, while Metal presentation behavior barely changed. | Record the delta, but add no Alpine task. These changes serve remote and generalized execution concerns outside the v1 boundary. |
| CS-WGPU-011 | WGPU v30 moved presentation ownership to `Queue::present`, added structured surface capabilities such as color spaces, and added automatic staging recall tied to submission. | Treat queue-owned present as a useful ownership comparison, not an API target. Revisit HDR only after SDR editor qualification. |
| CS-WGPU-012 | WGPU's published architecture acknowledges that `wgpu-hal` safety requirements are complex and not fully documented. | Do not use `wgpu-hal` as a shortcut around Alpine's reviewed Metal boundary. Any experimental oracle should use safe `wgpu`, not HAL internals. |

## Correctness impact

The strongest reusable idea is not a type or API. It is the separation of
contract evidence by layer. WGPU can validate generalized API behavior without
a GPU through its noop backend, then reserve real hardware for driver,
presentation, and image behavior. Alpine should apply the same testing economy
to its narrower state machines:

- pure models prove frame-slot, revision, cache, and recovery invariants;
- CPU and semantic oracles prove scene meaning;
- native Metal tests prove AppKit, drawable, shader, and driver behavior;
- product process tests prove files, input, IME, close, and restoration;
- fixed-hardware qualification begins only after all semantic gates pass.

This is a better use of WGPU than adopting it as a renderer. It increases the
strength of Alpine's gates without widening Alpine's runtime.

## Performance impact

WGPU does not prove Alpine performance, and this review found no upstream
result that can support a comparative Alpine claim. It does expose specific
performance hazards and measurement boundaries:

- normal presentation must remain completion-asynchronous;
- drawable acquisition must be visibility-aware;
- small uploads should reuse storage rather than allocate per write;
- callbacks must do minimal work because progress can be driven by polling;
- adapter, validation, state tracking, shader translation, and abstraction
  costs must be excluded from renderer-only comparisons only when both sides
  begin from equivalent prepared scenes;
- a WGPU differential run is correctness evidence unless the comparator
  protocol separately admits it as a performance workload.

No performance task is created from source inspection alone. The experiments
in the [research protocol](../research/wgpu/experiments.md) must produce raw,
revision-bound evidence first.

## Memory-efficiency impact

WGPU offers useful lifetime and staging patterns but not a directly comparable
editor memory budget. WGPU tracks many resource classes and generalized command
states that Alpine intentionally omits. Therefore:

- compare process footprint only for an explicitly equal workload;
- report adapter and translation setup separately from steady-state rendering;
- retain exact Alpine-owned GPU, atlas, upload, scene, and cache bytes;
- treat capacity retained after cancellation, failed submission, occlusion, or
  close as a correctness defect before treating it as a performance metric;
- never claim that a smaller Alpine structure is more efficient if it performs
  less validation or produces different output.

## Delivery impact

The research narrows delivery rather than adding a WGPU workstream. The direct
Metal daily-driver remains the critical path. The only immediate additions are:

1. Retain WGPU's failure classes as test-design input for presentation and
   upload ownership.
2. Add dependency-tree and excluded-feature checks to the no-bloat gate.
3. Prepare a separately approved lab adapter only when Alpine's glyph and code
   viewport traces are stable enough for a meaningful differential oracle.
4. Re-evaluate WGPU after the Metal daily-driver gate or when a non-Apple
   backend becomes active.

## Explicit non-decisions

- No WGPU dependency is approved.
- No `wgpu-hal` dependency is approved.
- No WebGPU-compatible Alpine public API is approved.
- No WGSL or Naga shipping path is approved.
- No Vulkan, D3D12, GLES, browser, remote-core, or multi-adapter work moves
  ahead of the macOS daily-driver gate.
- No WGPU timing or memory superiority claim exists.

## Deep evidence

- [Research package index](../research/wgpu/index.md)
- [Pinned primary-source map](../research/wgpu/source-map.md)
- [Detailed findings and adversarial analysis](../research/wgpu/findings.md)
- [Differential and lifecycle experiment protocol](../research/wgpu/experiments.md)
- [Include, investigate, and reject decisions](../research/wgpu/decisions.md)
