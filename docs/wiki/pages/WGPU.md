# WGPU role and boundary

> Retrieval mirror synchronized from Alpine `main` revision
> `{{ALPINE_MAIN_REVISION}}`. The repository mdBook is canonical.

The prior four-bullet summary has been replaced by a revision-pinned research
package. The reviewed source identity is
`8ee190c6f151c731a4f8cfd9a102d6ee5903460a`, with `v30.0.0` as release context.

WGPU is factored into Alpine as:

- a source of portable GPU API and validation lessons;
- a candidate independent readback and differential correctness oracle;
- a possible later portability backend, subject to a separate accepted AEP;
- an upstream whose exact evidence and revision must be retained.

The immediate reusable lessons are completion-owned resource lifetimes,
structured surface outcomes, occlusion-aware drawable admission, bounded
staging reuse, fast no-GPU validation, real-GPU behavior tests, tolerant image
comparison, and dependency-tree tests. Alpine copies those obligations, not the
portable WebGPU architecture.

WGPU is not a v1 shipping dependency or the Apple Silicon fast path. Alpine's
owned direct Metal backend remains the v1 renderer. Shipping WGPU later requires
dependency review, semantic parity, lifecycle evidence, and measured startup,
latency, binary-size, memory, and adaptation overhead. Research inclusion alone
does not authorize architecture or dependency changes.

Canonical sources: [WGPU case study](https://github.com/dbuddha/alpine-gpui/blob/{{ALPINE_MAIN_REVISION}}/docs/case-studies/wgpu.md) and [deep research package](https://github.com/dbuddha/alpine-gpui/blob/{{ALPINE_MAIN_REVISION}}/docs/research/wgpu/index.md)
