# WGPU

- Reviewed: 2026-08-13
- Research: [#23](https://github.com/dbuddha/alpine-gpui/issues/23)
- Revision: [`ee5cfb074fd0c4e318b5f8608df504678e4e17ac`](https://github.com/gfx-rs/wgpu/tree/ee5cfb074fd0c4e318b5f8608df504678e4e17ac)
- License: Apache-2.0 or MIT by repository policy
- Influence: conceptual, validation-oriented, and potentially differential

## Findings

- **CS-WGPU-001:** Explicit validation and capability discovery are useful
  specimens for structured errors and portable conformance tests.
- **CS-WGPU-002:** Backend-neutral scene semantics can be compared through CPU
  oracles and tolerant offscreen readback without exact cross-GPU hashes.
- **CS-WGPU-003:** A mature portability layer still cannot define Alpine's
  direct Metal resource lifetime, fast paths, or fixed-hardware budgets.
- **CS-WGPU-004:** WGPU can become an optional differential oracle only behind
  an Alpine-owned boundary and a separately approved dependency decision.

No WGPU source or dependency is incorporated by this case study.
