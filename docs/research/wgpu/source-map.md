# WGPU pinned source map

All repository links in this table use reviewed revision
`8ee190c6f151c731a4f8cfd9a102d6ee5903460a` unless explicitly marked as
historical or release material. Retrieval date is 2026-08-18.

## Repository and release identity

| ID | Source | Evidence class | What it supports | Limitation |
| --- | --- | --- | --- | --- |
| WGPU-S001 | [Repository README](https://github.com/gfx-rs/wgpu/blob/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/README.md) | Primary source | Supported backend and product scope | Overview, not a lifecycle contract |
| WGPU-S002 | [Repository architecture instructions](https://github.com/gfx-rs/wgpu/blob/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/AGENTS.md#L42) | Primary source | Roles of `wgpu`, `wgpu-core`, `wgpu-hal`, Naga, and CTS | Contributor-oriented and partly informal |
| WGPU-S003 | [Official Architecture Wiki](https://github.com/gfx-rs/wgpu/wiki/Architecture) | Primary source, mutable | Safety, lifetime, synchronization, barriers, initialization, tracing | Wiki is not revision-pinned; pinned code wins on conflict |
| WGPU-S004 | [v30.0.0 release](https://github.com/gfx-rs/wgpu/releases/tag/v30.0.0) | Primary source | Queue-owned present, staging recall, HDR, locking, Metal changes | Release summary, not full implementation |
| WGPU-S005 | [Historical Alpine pin](https://github.com/gfx-rs/wgpu/tree/ee5cfb074fd0c4e318b5f8608df504678e4e17ac) | Primary source | Prior research identity | Superseded for current findings, never erased |
| WGPU-S006 | [Current reviewed pin](https://github.com/gfx-rs/wgpu/tree/8ee190c6f151c731a4f8cfd9a102d6ee5903460a) | Primary source | Current research identity | Snapshot, not a claim about future releases |
| WGPU-S007 | [Remote-core extraction commit](https://github.com/gfx-rs/wgpu/commit/5cd735daf) | Primary source | Major delta after the historical pin | Mostly outside Alpine v1 scope |
| WGPU-S008 | [Locking migration commit](https://github.com/gfx-rs/wgpu/commit/8573e7a44) | Primary source | Synchronization delta after the historical pin | Does not establish performance impact |

## Public API and ownership

| ID | Source | Evidence class | What it supports | Limitation |
| --- | --- | --- | --- | --- |
| WGPU-S010 | [`Instance` and safe surface creation](https://github.com/gfx-rs/wgpu/blob/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/wgpu/src/api/instance.rs#L179) | Primary source | Surface target ownership and safe/unsafe boundary | Cross-platform API differs from AppKit-owned Alpine surface |
| WGPU-S011 | [`Surface` configuration and acquisition](https://github.com/gfx-rs/wgpu/blob/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/wgpu/src/api/surface.rs#L100) | Primary source | Configuration preconditions and structured acquire results | Public wrapper hides backend-specific timing |
| WGPU-S012 | [`Queue::submit` and completion callback](https://github.com/gfx-rs/wgpu/blob/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/wgpu/src/api/queue.rs#L279) | Primary source | Submission identity and asynchronous work completion | Progress requires submit or polling elsewhere |
| WGPU-S013 | [`Device::poll`](https://github.com/gfx-rs/wgpu/blob/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/wgpu/src/api/device.rs) | Primary source | Explicit progress and wait boundary | Browser behavior differs from native behavior |
| WGPU-S014 | [`StagingBelt`](https://github.com/gfx-rs/wgpu/blob/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/wgpu/src/util/belt.rs#L12) | Primary source | Ring-style staging reuse and recall lifecycle | General utility; no hard total-memory cap |

## Core lifetime and presentation

| ID | Source | Evidence class | What it supports | Limitation |
| --- | --- | --- | --- | --- |
| WGPU-S020 | [`LifetimeTracker`](https://github.com/gfx-rs/wgpu/blob/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/wgpu-core/src/device/life.rs#L156) | Primary source | Active submissions, mapping readiness, and callback ordering | Generalized WebGPU resources exceed Alpine's state space |
| WGPU-S021 | [Queue maintenance and submission](https://github.com/gfx-rs/wgpu/blob/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/wgpu-core/src/device/queue.rs#L235) | Primary source | Completion triage, submission indices, cleanup | Locking and registries are architecture-specific |
| WGPU-S022 | [Core surface acquisition](https://github.com/gfx-rs/wgpu/blob/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/wgpu-core/src/present.rs#L159) | Primary source | Configured-device check, bounded acquire, structured status | Fixed timeout policy is not an Alpine recommendation |
| WGPU-S023 | [Resource tracking module](https://github.com/gfx-rs/wgpu/blob/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/wgpu-core/src/track/mod.rs) | Primary source | State tracking and retention breadth | Much of this breadth exists for Vulkan, D3D12, and untrusted inputs |

## Metal backend

| ID | Source | Evidence class | What it supports | Limitation |
| --- | --- | --- | --- | --- |
| WGPU-S030 | [Metal surface configuration and acquire](https://github.com/gfx-rs/wgpu/blob/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/wgpu-hal/src/metal/surface.rs#L253) | Primary source | Drawable count, timeout policy, occlusion check, `nextDrawable` | Uses `CAMetalLayer`, not Alpine's display-link-owned drawable path |
| WGPU-S031 | [Metal queue submit and present](https://github.com/gfx-rs/wgpu/blob/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/wgpu-hal/src/metal/mod.rs#L680) | Primary source | Completion handler, fence retention, commit, present, idle wait | Exact behavior differs when `presentsWithTransaction` is enabled |
| WGPU-S032 | [Metal fence wait](https://github.com/gfx-rs/wgpu/blob/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/wgpu-hal/src/metal/device.rs#L1995) | Primary source | Device-loss classification and timeout-aware waiting | Not a frame-loop design |
| WGPU-S033 | [Metal resource counters](https://github.com/gfx-rs/wgpu/blob/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/wgpu-hal/src/metal/device.rs) | Primary source | Per-resource counter discipline | Does not provide an editor-wide footprint budget |

## Testing and qualification

| ID | Source | Evidence class | What it supports | Limitation |
| --- | --- | --- | --- | --- |
| WGPU-S040 | [Testing guide](https://github.com/gfx-rs/wgpu/blob/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/docs/testing.md) | Primary source | Benchmark, image, snapshot, compile, dependency, validation, GPU, trace, and CTS test classes | Describes WGPU's suite, not Alpine's acceptance thresholds |
| WGPU-S041 | [Noop validation tests](https://github.com/gfx-rs/wgpu/tree/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/tests/tests/wgpu-validation) | Primary source | Fast validation without real hardware | Cannot prove driver or presentation behavior |
| WGPU-S042 | [Real-GPU tests](https://github.com/gfx-rs/wgpu/tree/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/tests/tests/wgpu-gpu) | Primary source | Behavior across available adapters and validation layers | Hardware matrix and expectations differ from Alpine's fixed-hardware protocol |
| WGPU-S043 | [Command-buffer action tests](https://github.com/gfx-rs/wgpu/blob/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/tests/tests/wgpu-validation/api/command_buffer_actions.rs#L81) | Primary source | Deferred callback order and submit admission | No real GPU in noop mode |
| WGPU-S044 | [Naga validation sources](https://github.com/gfx-rs/wgpu/tree/8ee190c6f151c731a4f8cfd9a102d6ee5903460a/naga/src/valid) | Primary source | Shader validation scope | Alpine uses compiled Metal libraries and does not need Naga in v1 |

## Source-use rules

- A pinned code link can support a statement about that revision only.
- The mutable Wiki explains intent but cannot override pinned implementation.
- Release notes identify user-visible changes but do not prove cost or safety.
- Issue and discussion text can identify a question, never a measured Alpine
  conclusion by itself.
- No private implementation claim is present in this package.
