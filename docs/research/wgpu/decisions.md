# WGPU include, investigate, and reject decisions

This decision ledger prevents useful research from becoming accidental product
scope. "Include" means include the pattern or test obligation in Alpine's plan,
not copy WGPU source.

## Include now

| ID | Decision | Alpine destination | Reversal condition |
| --- | --- | --- | --- |
| WGPU-D001 | Keep safe public contracts separate from native unsafe obligations. | Existing core, scene, renderer, and platform boundaries | A narrower boundary proves safer and remains independently testable |
| WGPU-D002 | Retain resources by submission and slot identity until terminal completion. | Bounded asynchronous presentation and upload ownership | Native API provides stronger terminal ownership evidence |
| WGPU-D003 | Treat visibility and occlusion as drawable-admission preconditions. | macOS lifecycle and native E2E | AppKit or Metal contract changes with evidence |
| WGPU-D004 | Keep surface failures structured and recovery-specific. | Presentation error and recovery contracts | None without a public API decision |
| WGPU-D005 | Test abandoned staging and unsubmitted work for bounded release. | Frame-slot, upload, close, and fault-injection gates | Ownership model removes prepared-but-unsubmitted state |
| WGPU-D006 | Separate no-device validation, real-GPU behavior, image, compile, dependency, and benchmark evidence. | CI classifier and assurance policy | A test class is proven redundant by stronger evidence |
| WGPU-D007 | Audit the dependency tree for excluded product systems. | No-bloat gate under Requirement #36 | Product boundary changes through owner-approved requirement |

## Investigate later

| ID | Candidate | Required evidence before action |
| --- | --- | --- |
| WGPU-D020 | Safe WGPU differential oracle | WGPU-X001 and WGPU-X005 task, dependency record, exact feature lock, no shipping reachability |
| WGPU-D021 | WGPU performance comparator | Correctness-qualified adapter, A/A calibration, stage-separated protocol, fixed hardware |
| WGPU-D022 | WGPU as non-Apple backend | Daily-driver Metal qualification, accepted platform requirement, semantic parity, startup and memory budget |
| WGPU-D023 | HDR or wide-gamut surface behavior | Stable SDR editor, display capability contract, color oracle, native qualification |
| WGPU-D024 | General staging arena | Evidence that Alpine's three-slot uploads are a bottleneck and a hard total-capacity design |

## Reject for v1

| ID | Rejected choice | Reason |
| --- | --- | --- |
| WGPU-D040 | WGPU as Alpine's Apple renderer | Duplicates and obscures the direct-Metal ownership and performance target |
| WGPU-D041 | `wgpu-hal` as a shortcut | Unsafe, broad, and documented by WGPU as complex; does not simplify Alpine's proof burden |
| WGPU-D042 | WebGPU-compatible public API | Introduces generalized states and conformance obligations Alpine Studio does not need |
| WGPU-D043 | Naga, WGSL, or runtime shader translation | Alpine v1 ships reviewed compiled Metal libraries and has no cross-backend shader need |
| WGPU-D044 | Remote-core registries or generalized IDs | Serves remote and multi-client architectures excluded from local Studio v1 |
| WGPU-D045 | Browser, GLES, Vulkan, or D3D12 work before daily-driver | Moves effort away from the Apple-first acceptance gate |
| WGPU-D046 | Exact cross-GPU pixel hashes | Driver and raster differences require semantic and tolerance-aware comparison |
| WGPU-D047 | Performance conclusions from source shape | Only controlled measurements can support latency or memory claims |

## No-copy and licensing boundary

This package uses conceptual, behavioral, workload, and differential influence
only. No WGPU source has been copied or adapted. Any future source adaptation
requires file-level provenance and license review in addition to the dependency
and architecture approval required by Alpine policy.
