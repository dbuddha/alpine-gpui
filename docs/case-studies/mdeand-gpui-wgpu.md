# mdeand gpui-wgpu

- Reviewed: 2026-08-09
- Research: [#22](https://github.com/dbuddha/alpine-gpui/issues/22)
- Revision: [`a2158ca36a0f46b32c3a66423b6498a3f0ed6ae1`](https://github.com/mdeand/gpui-wgpu/tree/a2158ca36a0f46b32c3a66423b6498a3f0ed6ae1)
- License: LICENSE-APACHE and crate declaration Apache-2.0
- Influence: conceptual, behavioral, and differential

## Findings

- **CS-MWGPU-001:** Surface registries and embedded GPU surfaces clarify which
  owner creates, retains, resizes, and destroys presentation resources.
- **CS-MWGPU-002:** A reported continuous-redraw regression on Apple Silicon
  makes zero idle submission a mandatory Alpine requirement.
- **CS-MWGPU-003:** Demand-driven wakeup and frame coalescing must be verified as
  lifecycle behavior and then measured on qualified hardware.

The reported idle improvement was not a controlled benchmark. It motivates
tests and a future performance budget but is not itself performance evidence.
