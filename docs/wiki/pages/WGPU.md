# WGPU role and boundary

> Retrieval mirror synchronized from Alpine `main` revision
> `{{ALPINE_MAIN_REVISION}}`. The repository mdBook is canonical.

WGPU is factored into Alpine as:

- a source of portable GPU API and validation lessons;
- a candidate independent readback and differential correctness oracle;
- a possible later portability backend, subject to a separate accepted AEP;
- an upstream whose exact evidence and revision must be retained.

WGPU is not a v1 shipping dependency or the Apple Silicon fast path. Alpine's
owned direct Metal backend remains the v1 renderer. Shipping WGPU later requires
dependency review, semantic parity, lifecycle evidence, and measured startup,
latency, binary-size, memory, and adaptation overhead. Research inclusion alone
does not authorize architecture or dependency changes.

Canonical source: [WGPU case study](https://github.com/dbuddha/alpine-gpui/blob/{{ALPINE_MAIN_REVISION}}/docs/case-studies/wgpu.md)
