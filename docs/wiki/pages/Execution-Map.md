# Alpine Studio execution map

> Retrieval mirror synchronized from Alpine `main` revision
> `{{ALPINE_MAIN_REVISION}}`. The repository mdBook is canonical.

Current status at the source revision:

1. Native lifecycle and a real production-path window: implemented.
2. Non-blocking, bounded Metal presentation: implemented.
3. A correct interactive one-file editor: functionally implemented, dogfood
   acceptance remains open.
4. A local workspace shell with bounded indexing and restoration: functionally
   implemented, repository-scale dogfood remains open.
5. Rust-first completion, navigation, configuration, native accessibility,
   assurance closure, and dogfooding: active critical path.
6. Scoped renderer and editor qualification: follows correctness and dogfood.

Correctness precedes performance. Performance precedes memory and resource
tuning. Delivery speed is optimized inside those constraints.

M5 is the selected Apple Silicon macOS daily-driver behavior and dogfood gate.
M7 is the separate supported, packaged, fixed-hardware-qualified version 1
gate. M6 is not on the macOS critical path. Milestone and Project item totals
must not be interpreted as readiness percentages; the canonical daily-driver
document defines leaf-only progress and exact exit evidence.

Canonical sources: [daily-driver path](https://github.com/dbuddha/alpine-gpui/blob/{{ALPINE_MAIN_REVISION}}/docs/use-cases/alpine-studio-highfidelity.md) and [adversarial review](https://github.com/dbuddha/alpine-gpui/blob/{{ALPINE_MAIN_REVISION}}/docs/research/alpine-studio-adversarial-review.md)
