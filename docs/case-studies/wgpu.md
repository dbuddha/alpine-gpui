# WGPU: portability, validation, and Metal comparison specimen

## Research record

| Field | Value |
| --- | --- |
| Status | Decision-grade case study |
| Research issue | [#23](https://github.com/dbuddha/alpine-gpui/issues/23) |
| Upstream | [gfx-rs/wgpu](https://github.com/gfx-rs/wgpu) |
| Examined revision | [`ee5cfb074fd0c4e318b5f8608df504678e4e17ac`](https://github.com/gfx-rs/wgpu/tree/ee5cfb074fd0c4e318b5f8608df504678e4e17ac) |
| Revision date | 2026-08-13 |
| Review date | 2026-08-17 |
| License | Apache-2.0 OR MIT |
| Evidence | Pinned source, official API documentation, upstream benchmark source |
| Alpine decision | Do not add WGPU to the v1 shipping path. Retain it as a pinned architecture specimen and candidate isolated differential renderer after an approved requirement and dependency decision. |

## Executive decision

WGPU is factored into Alpine's plan, but it is not the implementation of Alpine
GPUI. WGPU provides a safe, portable WebGPU-style API over Metal, Vulkan,
Direct3D 12, and related backends. Alpine v1 is an Apple-first UI runtime whose
performance thesis depends on a narrow scene model, direct ownership of one
`CAMetalLayer`, demand-driven frame admission, bounded resources, and native
lifecycle evidence.

Adding WGPU to the shipping path now would add a public API layer, validation and
resource tracking, shader translation, backend selection, and a generic surface
contract before Alpine needs portability. Those facilities serve WGPU's mission,
but do not advance the shortest correct path to a daily-driver editor.

Alpine should use WGPU in three constrained ways:

1. As a source of design patterns for capability discovery, structured failures,
   completion-indexed resource retirement, and benchmark separation.
2. As a negative-design specimen for genericity Alpine does not need in v1.
3. As a future isolated Metal-backed differential renderer for matched offscreen
   scene traces, only after Alpine's direct Metal path is independently correct.

WGPU is not GPUI, an editor framework, a text system, or an AppKit event loop. A
WGPU comparison must be reported as direct Metal versus WGPU-on-Metal for a named
renderer workload, never as a substitute for a Zed product comparison.

## Questions and method

This study asks:

1. What layers does WGPU add between an application and Metal?
2. Which layers improve correctness, portability, and observability?
3. Which layers create work or retained state Alpine's narrow v1 can avoid?
4. What does WGPU's Metal path do at the pinned revision?
5. Which benchmark techniques are reusable, and what do they fail to prove?
6. What exact role should WGPU have in Alpine's architecture and qualification?

**Fact** identifies claims supported by pinned source or current official
documentation. **Alpine inference** identifies a project conclusion. Current API
documentation is listed separately because it may postdate the source pin. No
conclusion relies on third-party summaries or benchmark headlines.

## Architecture at the pinned revision

### Public `wgpu` API

**Fact.** WGPU exposes a safe Rust API modeled on WebGPU and dispatches to native
graphics backends including Metal. Public objects cover adapters, devices,
queues, surfaces, command encoders, pipelines, buffers, textures, bind groups,
and completion callbacks. Naga translates WGSL for native backends.

**Alpine inference.** This object model is broader than Alpine needs. Alpine v1
has a small immutable UI primitive set, one Metal path, one native window owner,
and explicit frame reports. Reproducing WGPU's object model would make Alpine's
invalid-state surface larger without delivering editor behavior.

### `wgpu-core`

**Fact.** WGPU's HAL documentation says the safe core performs validation and
resource tracking omitted by the HAL. Pinned source contains submission indices,
resource lifetime tracking, delayed cleanup, callback delivery, and queue
maintenance around completed work.

**Alpine inference.** A general API needs those mechanisms for arbitrary resource
graphs. Alpine can preserve the essential safety property with three frame slots,
monotonic generation and frame tokens, ownership until completion, and stale
completion rejection. A general resource hub is not required.

### `wgpu-hal`

**Fact.** `wgpu-hal` is an unsafe cross-platform abstraction intended to minimize
translation overhead. It supports static dispatch and backend-specific access,
but its own pinned README warns that its safety requirements are not fully
documented.

**Alpine inference.** The HAL is useful source material, not a boundary to copy.
Alpine's native boundary should be smaller, document every unsafe invariant
locally, and expose no Metal handles through safe public APIs.

### Metal backend

**Fact.** The pinned backend configures `CAMetalLayer`, sets drawable size and
display synchronization, acquires `CAMetalDrawable`, and carries the drawable
through presentation. Device code selects shared, private, or memoryless Metal
storage according to resource use. It also contains generic allocator and hazard
tracking choices.

**Alpine inference.** This validates relevant Rust-to-Metal techniques, but does
not replace Alpine's AppKit contract. Alpine still owns `CAMetalDisplayLink`
deadlines, visibility, focus, input, close propagation, latest-wins invalidation,
and deterministic shutdown.

## Pinned Metal execution trace

1. The public API records surface, resource, pipeline, and command requests.
2. `wgpu-core` resolves resource identities, validates usage, tracks transitions,
   validates queue submission, and assigns submission progress.
3. The Metal backend obtains a drawable and creates or reuses native resources
   with the required storage modes.
4. Encoded Metal command buffers are committed through the queue path.
5. Completion advances queue progress, permits delayed destruction, and makes
   submitted-work callbacks eligible.
6. Presentation hands the acquired drawable to the display system.

These are distinct stages. Adaptation, WGPU validation, encoding, submission, GPU
completion, and presentation must not be combined into one timing number.

## Correctness mechanisms to retain

### Capabilities and structured failure

**Fact.** WGPU exposes adapters, features, limits, surface capabilities, and
fallible operations rather than assuming universal support.

**Decision.** Keep Alpine's narrower typed capabilities and errors. Unsupported
formats, unavailable or lost devices, missing drawables, allocation failure, and
invalid lifecycle transitions must remain structured errors, never panics.

### Completion-indexed lifetime

**Fact.** Pinned `wgpu-core` associates resources and callbacks with submission
progress and releases work only after the relevant submission completes.

**Decision.** Apply the property, not the general machinery. A frame slot remains
owned until terminal command completion. Handlers carry surface-generation and
frame tokens. Stale handlers may release slots but may not publish current-frame
success.

### Safe validation over an unsafe backend

**Fact.** WGPU separates safe validation and tracking from an unsafe low-level
backend.

**Decision.** Preserve Alpine's safe constructors and immutable scenes. Validate
only states Alpine can express, keep native objects private, and prove output with
an independent CPU oracle.

### Observable progress

**Fact.** WGPU makes submitted-work completion and device maintenance observable.

**Decision.** Report bounded in-flight work, terminal completion, retained bytes,
omissions, and shutdown drain in handle-free Alpine reports. Do not expose a
generic polling API to Studio.

## Performance cost model

These are not assumed defects. They are costs paid for WGPU's safe, portable,
general contract and must be measured rather than guessed.

| Cost center | WGPU purpose | Alpine v1 position |
| --- | --- | --- |
| API-to-core dispatch | Safe API over multiple backends | Avoid in the direct Metal shipping path |
| Resource identity and lookup | Own arbitrary GPU objects safely | Replace with typed frame-local ownership |
| Usage validation and tracking | Prevent invalid transitions | Validate Alpine's narrow scene before encode |
| Pipeline and binding generality | Support arbitrary WebGPU workloads | Use primitive-specific UI pipelines |
| WGSL and Naga | Portable shaders | Ship reviewed Apple-first Metal shader functions |
| Backend selection | Portability | Compile the supported Metal path in v1 |
| Surface abstraction | Portable presentation | Retain Alpine's AppKit display-link owner |
| Lifetime bookkeeping | Safe asynchronous completion | Use three slots and monotonic tokens |
| Staging allocation | Efficient general uploads | Use bounded reusable UI upload buffers |

**Alpine inference.** Alpine's likely advantage is deleting categories of work
through a narrower semantic model. This remains a hypothesis until matched traces
separate adaptation, validation, encode, submit, completion, and presentation.

## Memory and residency

### Storage modes

**Fact.** Pinned Metal device code selects shared, private, or memoryless storage.
WGPU exposes allocator reports where a backend supports them.

**Decision.** Record storage mode and exact Alpine-owned bytes for frame resources,
atlases, and caches. Rust allocator data alone cannot prove Metal residency.

### Deferred destruction

**Fact.** `wgpu-core` retains resources until using submissions complete. Logical
destruction and physical release therefore happen at different times.

**Decision.** Report current and peak slot bytes, completion lag, post-close drain,
and post-shutdown footprint delta. Dropping an application object is not proof of
release.

### Staging belt

**Fact.** WGPU's `StagingBelt` suballocates upload chunks, moves them through
active, closed, and free collections, and recalls them after submission. Its
source warns that `finish` without submission can permit indefinite allocation.

**Decision.** Adopt reuse but reject unbounded growth. Alpine uses a fixed slot
count, geometric growth only to an approved ceiling, exact byte accounting, and
release of oversized buffers after pressure or sustained disuse.

### Measurement limit

Allocator reports, physical footprint, Metal labels, and Rust allocation profiles
observe overlapping but different memory. Report each separately. Never claim
exact WGPU GPU residency unless the backend exposes and validates it.

## What upstream benchmarks prove

Pinned WGPU benchmark source contains useful controls:

- resource collections are shuffled to avoid measuring only a favorable order;
- command encoding and queue submission are timed separately;
- explicit device polling is outside the encoded-work duration;
- loops have explicit duration and iteration controls;
- unsupported virtualized GPU environments are called out.

Those benchmarks characterize selected WGPU operations. They do not prove editor
latency, AppKit lifecycle correctness, presentation time, text layout performance,
or whole-product memory efficiency. Alpine may reuse the controls, not cite their
numbers as evidence for an Alpine claim.

## Adopt, adapt, reject, defer

| Disposition | Lesson or facility | Reason |
| --- | --- | --- |
| Adopt | Explicit capabilities and structured errors | Improves correctness without widening the product |
| Adopt | Completion-indexed retirement | Prevents premature reuse |
| Adopt | Stage-separated benchmark evidence | Keeps adaptation and waits visible |
| Adopt | Allocation and lifetime reporting | Makes memory claims auditable |
| Adopt | Fixed revision and backend identity | Makes results reproducible |
| Adapt | Lifetime tracker into three frame slots | Preserves safety with bounded UI state |
| Adapt | Staging belt into a hard-bounded upload ring | Retains reuse without unbounded allocation |
| Adapt | Validation into typed scene constructors and CPU oracles | Pays only for Alpine semantics |
| Adapt | API benchmarks into matched scene traces | Tests Alpine's actual claim |
| Reject for v1 | Shipping WGPU dependency | Adds portability before the editor needs it |
| Reject for v1 | WebGPU resource and binding model | Broader than the UI renderer |
| Reject for v1 | WGSL and runtime translation | Adds startup and failure surface without a requirement |
| Reject for v1 | Runtime backend selection | Conflicts with the Metal-first proving ground |
| Reject for v1 | WGPU surface as app owner | Does not own Alpine's AppKit semantics |
| Defer | Portable WGPU-backed Alpine renderer | Revisit after daily-driver qualification and an approved portability requirement |

## Optional differential renderer

WGPU may enter executable Alpine code only through an approved lab adapter. It
must not become a transitive dependency of shipping crates.

1. Decode one versioned `alpine-scene-trace` into a neutral prepared scene.
2. Feed that scene to direct Metal and WGPU forced to Metal.
3. Render offscreen first so surface scheduling does not contaminate semantics.
4. Compare both outputs against the CPU pixel oracle at the same tolerance.
5. Record adaptation, validation, upload, encode, submit, GPU completion, and
   readback as separate distributions.
6. Add onscreen presentation only after offscreen equivalence is green.

The adapter must be removable. No public Alpine API widens for WGPU concepts. A
semantic mismatch invalidates the run rather than weakening the workload.

## Fair-comparison protocol

### Controls

- Pin Alpine, WGPU, workload, shader, and adapter revisions.
- Force WGPU to Metal and record adapter, device, features, limits, and surface
  configuration.
- Match pixel format, dimensions, scale, color treatment, clipping, primitive
  order, glyph data, and frame count.
- Begin renderer timing only after identical prepared-scene semantics exist.
- Report cold and warm runs separately.
- Fix resource reuse and keep setup outside timed stages.
- Separate CPU submit, GPU completion, and presentation.
- Require correctness, lifecycle, and memory ceilings before speed claims.
- Use paired randomized AB/BA runs on fixed hardware and retain raw samples.

### Workloads

1. Solid quad.
2. Clipped quad grid.
3. Monochrome glyph grid.
4. Realistic code viewport with selection and caret overlays.
5. Small scroll delta.
6. Resize and drawable turnover, reported as lifecycle evidence rather than mixed
   into steady-state throughput.

### Metrics

| Stage | Required evidence |
| --- | --- |
| Adaptation | Duration, allocations, semantic hash |
| Validation | Duration and rejected-operation count |
| Upload | Bytes, allocation or reuse count, duration |
| Encode | CPU duration and primitive counts |
| Submit | CPU duration and in-flight depth |
| GPU completion | Latency and terminal status |
| Presentation | Deadline, presented time, omissions, drawable status |
| Memory | Footprint, private dirty, allocator data, known GPU bytes, caches, post-close delta |

### Invalid claims

- "Alpine is faster than WGPU" without workload, stage, revision, and hardware.
- "WGPU overhead is X percent" when adaptation or behavior differs.
- "WGPU uses X bytes of GPU memory" based only on process or Rust allocations.
- "Direct Metal is inherently faster" without matched output and confidence
  intervals.
- "WGPU proves Alpine is portable" when only Metal was exercised.

## Roadmap impact

WGPU adds no daily-driver prerequisite. It reinforces this critical path:

1. Finish production AppKit window, close, and deterministic run-loop gates.
2. Remove blocking GPU waits and qualify the bounded frame-slot ring.
3. Deliver text, input, IME, one-file editing, and atomic save.
4. Deliver workspace navigation and crash-safe restoration.
5. Finish language, settings, accessibility, and dogfood qualification.
6. Qualify direct Metal against GPUI and product journeys against Zed and Sublime.
7. Consider WGPU only for a specific portability or validation question that does
   not delay gates 1 through 6.

WGPU sharpens Alpine's plan instead of expanding it: retain correctness patterns
and experimental controls while declining a premature portability layer.

## Findings and durable decisions

Stable IDs are retained for issue, requirement, and future research references.

| ID | Finding | Decision |
| --- | --- | --- |
| `CS-WGPU-001` | WGPU portability is useful for comparison, not a v1 requirement | Keep direct Metal as shipping backend |
| `CS-WGPU-002` | Validation and tracking provide a differential correctness model | Adapt properties into Alpine contracts; consider an isolated oracle |
| `CS-WGPU-003` | WGPU can support portability experiments after Apple-first qualification | Defer shipping integration |
| `CS-WGPU-004` | WGPU cannot replace independent semantic, memory, or presentation evidence | Keep CPU, readback, lifecycle, and footprint evidence independent |
| `CS-WGPU-005` | Completion-indexed lifetime is the essential asynchronous reuse property | Use bounded slots with generation and frame tokens |
| `CS-WGPU-006` | Generic staging can grow without bound when submission protocol is violated | Enforce hard byte and slot ceilings |
| `CS-WGPU-007` | Upstream benchmarks separate encode, submit, and poll | Preserve stage separation |
| `CS-WGPU-008` | Public, core, HAL, Naga, and backend layers exceed Alpine UI needs | Do not reproduce the generic object and shader model |
| `CS-WGPU-009` | Metal surface handling does not provide AppKit application semantics | Keep lifecycle and input in `alpine-platform` |
| `CS-WGPU-010` | Fair comparison must force Metal and use identical prepared scenes | Reject generic cross-backend headlines |

## Open questions

- Can an isolated WGPU-on-Metal adapter find synchronization defects missed by the
  CPU oracle and direct Metal readback?
- What is measured validation cost for a realistic code viewport after both
  adapters receive the same prepared scene?
- Can WGPU expose enough backend allocation evidence for useful GPU-residency
  comparison without an invasive fork?
- Would a future non-Apple requirement justify WGPU, or would another narrow
  native backend remain simpler?

Each question needs a new accepted task with a falsifiable gate. None authorizes a
shipping dependency.

## Primary source catalog

### Pinned revision

- [README: native backends and scope](https://github.com/gfx-rs/wgpu/blob/ee5cfb074fd0c4e318b5f8608df504678e4e17ac/README.md#L8-L10)
- [README: WebGPU drafts and Naga](https://github.com/gfx-rs/wgpu/blob/ee5cfb074fd0c4e318b5f8608df504678e4e17ac/README.md#L137-L164)
- [`wgpu-hal` architecture and safety boundary](https://github.com/gfx-rs/wgpu/blob/ee5cfb074fd0c4e318b5f8608df504678e4e17ac/wgpu-hal/README.md#L1-L37)
- [`wgpu-hal` safety-contract warning](https://github.com/gfx-rs/wgpu/blob/ee5cfb074fd0c4e318b5f8608df504678e4e17ac/wgpu-hal/README.md#L65-L70)
- [Core queue submission and callbacks](https://github.com/gfx-rs/wgpu/blob/ee5cfb074fd0c4e318b5f8608df504678e4e17ac/wgpu-core/src/device/queue.rs)
- [Core lifetime tracker](https://github.com/gfx-rs/wgpu/blob/ee5cfb074fd0c4e318b5f8608df504678e4e17ac/wgpu-core/src/device/life.rs)
- [Core resource submission state](https://github.com/gfx-rs/wgpu/blob/ee5cfb074fd0c4e318b5f8608df504678e4e17ac/wgpu-core/src/resource.rs)
- [Metal surface and drawable presentation](https://github.com/gfx-rs/wgpu/blob/ee5cfb074fd0c4e318b5f8608df504678e4e17ac/wgpu-hal/src/metal/surface.rs)
- [Metal device and storage modes](https://github.com/gfx-rs/wgpu/blob/ee5cfb074fd0c4e318b5f8608df504678e4e17ac/wgpu-hal/src/metal/device.rs)
- [Public queue and completion callbacks](https://github.com/gfx-rs/wgpu/blob/ee5cfb074fd0c4e318b5f8608df504678e4e17ac/wgpu/src/api/queue.rs)
- [Public allocator report](https://github.com/gfx-rs/wgpu/blob/ee5cfb074fd0c4e318b5f8608df504678e4e17ac/wgpu/src/api/device.rs#L630-L631)
- [`StagingBelt` lifecycle and growth warning](https://github.com/gfx-rs/wgpu/blob/ee5cfb074fd0c4e318b5f8608df504678e4e17ac/wgpu/src/util/belt.rs)
- [Resource-creation benchmark](https://github.com/gfx-rs/wgpu/blob/ee5cfb074fd0c4e318b5f8608df504678e4e17ac/benches/benchmarks/resource_creation.rs)
- [Render-pass benchmark](https://github.com/gfx-rs/wgpu/blob/ee5cfb074fd0c4e318b5f8608df504678e4e17ac/benches/benchmarks/renderpass.rs)
- [Bind-group benchmark](https://github.com/gfx-rs/wgpu/blob/ee5cfb074fd0c4e318b5f8608df504678e4e17ac/benches/benchmarks/bind_groups.rs)

### Current official API documentation

These may postdate the pin and are not proof of identical pinned behavior.

- [`SurfaceConfiguration`](https://docs.rs/wgpu/latest/wgpu/type.SurfaceConfiguration.html)
- [`MemoryHints`](https://docs.rs/wgpu/latest/wgpu/enum.MemoryHints.html)
- [`StagingBelt`](https://docs.rs/wgpu/latest/wgpu/util/struct.StagingBelt.html)
- [`Backend`](https://docs.rs/wgpu/latest/wgpu/enum.Backend.html)

## Requalification rule

This revision remains immutable. A later WGPU version requires a dated new record
or explicit revision-delta section covering architecture, Metal surfaces,
lifetimes, staging, benchmarks, and effects on Alpine decisions.
