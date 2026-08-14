# Nestri gpui-wgpu

- Reviewed: 2026-08-09
- Research: [#21](https://github.com/dbuddha/alpine-gpui/issues/21)
- Revision: [`49d46d31a14f2f11efe17a3157a0f0ef4c825bd4`](https://github.com/nestrilabs/gpui-wgpu/tree/49d46d31a14f2f11efe17a3157a0f0ef4c825bd4)
- License: LICENSE-APACHE and crate declaration Apache-2.0
- Influence: conceptual and differential

## Findings

- **CS-NWGPU-001:** A WGPU-backed GPUI lineage demonstrates that portable scene
  behavior can be separated from surface and backend ownership.
- **CS-NWGPU-002:** Fork lineage is useful for comparing architectural choices,
  but it creates maintenance and provenance costs rather than eliminating them.
- **CS-NWGPU-003:** A portable backend can be a differential oracle without
  becoming the authority for Metal lifetime or performance behavior.

This revision does not establish complete platform support, Metal-specific
performance, or a sustainable upstream-update strategy for Alpine.
