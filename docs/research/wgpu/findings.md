# WGPU detailed findings

The finding identifiers below are stable. Each finding separates a
primary-source observation from the Alpine inference that follows from it.

## Primary-source findings

### WGPU-F001: layered safety responsibilities

**Observation.** WGPU documents four principal layers: the idiomatic `wgpu`
API, the validated `wgpu-core`, the unsafe portable `wgpu-hal`, and native GPU
APIs. Naga translates and validates shader languages. `wgpu-core` takes
responsibility for lifetimes, synchronization, barriers, initialization,
cross-device checks, and generalized parameter validation.

**Sources.** WGPU-S002, WGPU-S003, WGPU-S023, WGPU-S044.

**Alpine inference.** Alpine should preserve its safe-value to unsafe-native
boundary and make every layer's obligations explicit. It should not reproduce
the portable HAL or generalized WebGPU state tracker because the owned scene
already constrains legal resource transitions far more tightly.

### WGPU-F002: surface ownership is a safety contract

**Observation.** Safe WGPU surface creation retains the handle source when
needed so the target outlives the surface. Configuration validates dimensions,
format, usage, view formats, latency, and color space. Acquisition produces
structured states instead of a Boolean success path.

**Sources.** WGPU-S010, WGPU-S011, WGPU-S022.

**Alpine inference.** `NativeSurface::run` should continue to own AppKit and
Metal surface lifetimes as one contract. Alpine should map every native failure
to a stable recovery class and test the exact close ordering. A portable window
handle abstraction would weaken rather than strengthen the v1 contract.

### WGPU-F003: completion owns resource release

**Observation.** WGPU assigns submission indices, retains active submission
resources, triages them after fence progress, and orders mapping completion
before submitted-work-done callbacks. Its Metal backend retains command buffers
against fence values and uses a completion handler to advance waiters.

**Sources.** WGPU-S012, WGPU-S020, WGPU-S021, WGPU-S031, WGPU-S043.

**Alpine inference.** Alpine's generation, frame token, revision, resize epoch,
and slot identity are the correct narrower equivalent. A callback may release
the slot it owns, but stale completion must not publish current-frame success.
Close and device loss must drain or classify every committed slot.

### WGPU-F004: normal Metal present is not a GPU-completion barrier

**Observation.** WGPU commits command buffers normally and uses completion
handlers for progress. Its normal present path commits a presentation command
buffer. The explicit `waitUntilCompleted` call is in `wait_for_idle`; a
transactional present waits until scheduled, not until GPU completion.

**Sources.** WGPU-S031, WGPU-S032.

**Alpine inference.** Alpine's display-link callback must encode, commit,
present, and return. A completion wait on that callback would serialize CPU and
GPU work and make input responsiveness hostage to GPU completion.

### WGPU-F005: occlusion is part of drawable admission

**Observation.** WGPU's Metal surface configures maximum drawable count as
requested frame latency plus one. On macOS, current source checks hosting-window
occlusion before `nextDrawable` because acquiring while occluded can stall.

**Sources.** WGPU-S030.

**Alpine inference.** Hidden, minimized, zero-sized, and occluded surfaces must
not acquire a drawable. Visibility is a correctness precondition for frame
admission, not a later performance optimization.

### WGPU-F006: reusable staging has an abandonment hazard

**Observation.** `StagingBelt` suballocates reusable staging chunks and recalls
them after submission completion. Its contract explicitly warns that if the
encoder is never submitted after automatic recall is armed, chunks are not
returned and new allocations can continue indefinitely.

**Sources.** WGPU-S004, WGPU-S014.

**Alpine inference.** Alpine should keep its simpler three-slot upload ownership
and hard capacity policy. Tests must cover failed encoding, canceled admission,
commit failure, occlusion after preparation, and close before completion. Each
path either reuses, sheds, or releases capacity with exact accounting.

### WGPU-F007: validation and hardware truth are separate suites

**Observation.** WGPU uses a noop backend for validation and trace tests,
real-GPU tests for actual behavior, tolerant image comparison for examples,
compile-fail tests for lifetime contracts, dependency-tree tests for feature
leaks, shader snapshots and validators, benchmarks, and WebGPU CTS. The testing
guide says trace/player tests are difficult and currently broken, so new tests
should prefer the supported harnesses.

**Sources.** WGPU-S040, WGPU-S041, WGPU-S042, WGPU-S043.

**Alpine inference.** Alpine's policy should select the cheapest test that can
prove each claim, but never let a model stand in for native truth. Dependency
tree and binary/process exclusion checks are particularly relevant to the
no-bloat product boundary.

### WGPU-F008: broad validation has intentional cost and value

**Observation.** WGPU validates an API intended to remain safe even when driven
by untrusted web content. It tracks states and lifetimes across generalized
resources and inserts transitions and zero initialization where required.

**Sources.** WGPU-S003, WGPU-S020, WGPU-S021, WGPU-S023.

**Alpine inference.** Comparing Alpine and WGPU without equalizing semantics is
invalid. Alpine can be smaller because it accepts fewer states, but it must
still reject every malformed state reachable through its public API. Doing less
work is only an efficiency result when the admitted workload is equal.

### WGPU-F009: v30 adds useful comparisons, not new Alpine scope

**Observation.** WGPU v30 moves presentation to `Queue::present`, adds automatic
staging recall tied to submission, expands surface color-space capabilities,
relaxes locking, and expands backend features. The reviewed post-baseline delta
also extracts global registries into `wgpu-core-remote` and changes locking
primitives.

**Sources.** WGPU-S004, WGPU-S007, WGPU-S008.

**Alpine inference.** Queue-owned presentation and deferred recall are useful
ownership comparisons. HDR, remote registries, custom backends, and broad
feature growth do not belong on Alpine's daily-driver path.

### WGPU-F010: resource counters are not total memory evidence

**Observation.** The Metal backend maintains counters around classes of native
resources, and core tracks live submissions and resources. These mechanisms do
not by themselves report total process physical footprint, allocator overhead,
driver allocations, every cache byte, or application residency.

**Sources.** WGPU-S020, WGPU-S021, WGPU-S033.

**Alpine inference.** Exact Alpine-owned accounting and operating-system
footprint measurements remain separate mandatory endpoints. Neither can
substitute for the other.

## Correctness review

WGPU's best correctness lesson is explicit obligation transfer. The safe layer
must know exactly which unsafe precondition it establishes, resource retention
must end only after terminal GPU evidence, and platform failures must remain
structured through public boundaries. Alpine already follows this direction.
The next gains come from testing presentation and upload abandonment with the
same seriousness as successful rendering.

The main rejected correctness shortcut is using WGPU as a second renderer and
calling agreement proof. Two GPU backends can agree while sharing an adapter
bug, a trace bug, or an underspecified output. Alpine still needs its independent
semantic scene oracle and CPU pixel oracle.

## Performance review

Source structure cannot establish relative speed. The findings identify only
mechanisms and hazards:

- completion waits are absent from WGPU's ordinary Metal present path;
- acquisition can block or time out, so visibility and stage timing matter;
- staging reuse can reduce allocation frequency for many small writes;
- generalized validation, state tracking, and shader translation are costs
  whose value depends on the workload;
- polling and callback execution can move work between threads and stages, so
  wall time must be attributed rather than hidden.

For Alpine, the relevant performance question is not "is direct Metal faster
than WGPU?" It is "for the same prepared scene and output, where do validation,
adaptation, upload, encode, completion, and presentation time differ?" The
experiment protocol preserves those endpoints.

## Memory-efficiency review

The WGPU lifetime tracker demonstrates why dropping a public handle cannot
imply immediate native release. Submission ownership, mapping, callbacks, and
backend fences all affect residency. The staging belt demonstrates a second
lesson: reuse reduces churn only when every terminal path returns the storage.

Alpine's narrower design can avoid generalized registries and trackers, but the
saved structure is not evidence until the same workload reaches the same final
state. Required endpoints are current and peak Alpine-owned bytes, process
physical footprint, private dirty memory, GPU resource bytes where observable,
steady-state slope, and post-close delta.

## Delivery review

Adding WGPU to the shipping graph now would slow the daily-driver path through
dependency review, adapter semantics, shader strategy, binary growth, duplicate
surface ownership, and a second performance path. The research therefore
creates no renderer implementation task.

The delivery-positive actions are limited: use its test taxonomy, add explicit
abandonment cases to existing bounded ownership work, and prepare a lab-only
differential adapter after scene semantics stabilize.

## Alpine inferences

1. WGPU belongs in the qualification architecture before it belongs in the
   product architecture.
2. Alpine's best advantage comes from a smaller admitted state space, not from
   removing validation.
3. Direct Metal remains justified for the first platform because it keeps
   display timing, resource ownership, and memory evidence under Alpine control.
4. A future portability backend must conform to Alpine scene semantics rather
   than redefine them around WebGPU.

## Unverified hypotheses

1. Alpine's prepared-scene CPU cost is lower than safe WGPU for matched code
   viewport workloads.
2. Alpine's three owned upload slots retain fewer steady-state bytes than a
   tuned WGPU staging strategy for the same dynamic instance stream.
3. Alpine's display-link-owned presentation has lower input-to-present latency
   variance than a conventional WGPU window loop on the same hardware.
4. WGPU readback can detect a useful class of Alpine Metal defects without
   introducing correlated shader or trace errors.

These hypotheses remain non-authoritative until the experiment protocol is run.
