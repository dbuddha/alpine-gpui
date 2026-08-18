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
5. Rust-first daily-driver behavior, settings, accessibility, and dogfooding:
   active critical path.
6. Scoped renderer and editor qualification: follows correctness and dogfood.

Correctness precedes performance. Performance precedes memory and resource
tuning. Delivery speed is optimized inside those constraints.

M5 in the current GitHub milestone scheme is a component and dogfood milestone.
It is not, by itself, the Alpine Studio daily-driver exit gate. The canonical
daily-driver document owns that gate, followed by applicable M7 release work.

Canonical sources: [daily-driver path](https://github.com/dbuddha/alpine-gpui/blob/{{ALPINE_MAIN_REVISION}}/docs/use-cases/alpine-studio-highfidelity.md) and [adversarial review](https://github.com/dbuddha/alpine-gpui/blob/{{ALPINE_MAIN_REVISION}}/docs/research/alpine-studio-adversarial-review.md)
