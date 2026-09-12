# Portable contracts and native specialization

Portable semantics stop at `Scene` and `Renderer`. Associated types prevent the
portable contract from dictating a target or error representation. A backend is
free to specialize formats, batching, atlases, synchronization, and
presentation while preserving observable behavior.

```mermaid
flowchart TB
    core["Portable value contracts<br/>alpine-core"]
    scene["Portable immutable scene<br/>alpine-scene"]
    contract["Portable renderer call and evidence<br/>alpine-renderer"]
    metal_plan["Direct Metal safe plan<br/>implemented"]
    metal_native["Direct Metal specialization<br/>offscreen readback and callback drawable implemented"]
    macos["Native macOS owner<br/>demand-driven callback presentation"]
    vulkan["Direct Vulkan specialization<br/>not implemented"]
    d3d12["Direct D3D12 specialization<br/>not implemented"]

    core --> scene --> contract
    scene --> metal_plan
    contract -->|"FrameReport type"| metal_native
    metal_plan --> metal_native
    macos -->|"target-only SPI"| metal_native
    contract -.-> vulkan
    contract -.-> d3d12
```

No portable abstraction may prevent a Metal-specific fast path. WGPU may be a
future differential oracle or optional compatibility backend, but it does not
define Metal behavior.
